#!/bin/bash
# Self-test of the HPC scripts against a fake cluster on this machine: fake ssh, squeue,
# sbatch, sacct, apptainer, rustup, rustc and cargo in a temporary folder. Runs no real
# cargo and contacts nothing. Run from the repository: scripts/hpc/selftest.sh
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
cd "$(git -C "$HERE" rev-parse --show-toplevel)" || exit 1
F=$(mktemp -d)
trap 'rm -rf "$F"' EXIT
mkdir -p "$F/bin" "$F/home/.cargo" "$F/state" "$F/remote" "$F/runs" "$F/plan/ctl"
echo "export PATH=\"$F/bin:\$PATH\"" >"$F/home/.cargo/env"

cat >"$F/ssh-local" <<EOF
#!/bin/bash
while [ \$# -gt 0 ]; do case "\$1" in -o) shift 2;; -*) shift;; *) break;; esac; done
shift
[ -e "$F/state/offline" ] && exit 255
if [ -e "$F/state/drop" ] && [[ "\$*" == *prepare.sh* ]]; then
    HOME="$F/home" PATH="$F/bin:\$PATH" APPTAINER="$F/bin/apptainer" bash -c "\$*" >/dev/null 2>&1; exit 255
fi
export HOME="$F/home" PATH="$F/bin:\$PATH" APPTAINER="$F/bin/apptainer"
exec bash -c "\$*"
EOF
cat >"$F/bin/squeue" <<EOF
#!/bin/bash
q="$F/state/queue"; [ -f "\$q" ] || exit 0
case " \$* " in
    *" -j "*) id=\$(echo "\$*" | sed -n 's/.*-j \([0-9]*\).*/\1/p'); grep "^\$id " "\$q" || true;;
    *" -n "*) nm=\$(echo "\$*" | sed -n 's/.*-n \([^ ]*\).*/\1/p'); awk -v n="\$nm" '\$2 == n' "\$q";;
    *) cat "\$q";;
esac
EOF
cat >"$F/bin/sbatch" <<EOF
#!/bin/bash
c="$F/state/counter"; id=\$(( \$(cat "\$c" 2>/dev/null || echo 30000) + 1 )); echo \$id > "\$c"
cpus=1; out=/dev/null; jn=job
for a in "\$@"; do case "\$a" in --cpus-per-task=*) cpus=\${a#*=};; --output=*) out=\${a#*=};; --job-name=*) jn=\${a#*=};; esac; done
script=\${@: -2:1}; run=\${@: -1}; out=\${out//%j/\$id}
if [ -e "$F/state/hold" ]; then echo "\$id \$jn PENDING" > "$F/state/queue"; echo "\$id|PENDING|0:0|00:00:00|0|8G||None|Unknown|Unknown" > "$F/state/sacct-\$id"; echo "\$id"; exit 0; fi
SLURM_JOB_ID=\$id SLURM_CPUS_PER_TASK=\$cpus bash "\$script" "\$run" > "\$out" 2>&1; ex=\$?
st=COMPLETED; [ \$ex -ne 0 ] && st=FAILED
echo "\$id|\$st|\$ex:0|00:00:03|\$cpus|8G|cpu=\$cpus,mem=8G,node=1|fakenode1|start|end" > "$F/state/sacct-\$id"
echo "\$id;fakecluster"
EOF
cat >"$F/bin/sacct" <<EOF
#!/bin/bash
id=\$(echo "\$*" | sed -n 's/.*-j \([0-9]*\).*/\1/p'); cat "$F/state/sacct-\$id" 2>/dev/null || true
EOF
printf '#!/bin/bash\nshift 2; exec "$@"\n' >"$F/bin/apptainer"
cat >"$F/bin/rustup" <<EOF
#!/bin/bash
case "\$1 \$2" in "toolchain list") [ -e "$F/state/no-toolchain" ] || echo "1.96.0-x86_64-unknown-linux-gnu";; "toolchain install") rm -f "$F/state/no-toolchain";; esac
EOF
cat >"$F/bin/rustc" <<EOF
#!/bin/bash
if [ -e "$F/state/wrong-rustc" ]; then echo "rustc 1.100.0 (fake)"; else echo "rustc 1.96.0 (fake)"; fi
EOF
cat >"$F/bin/cargo" <<'EOF'
#!/bin/bash
case "$1" in fetch) ;; -V) echo "cargo 1.96.0 (fake)" ;; *) exit 1 ;; esac
EOF
# curl serves a fake protoc release zip and logs each download; nothing leaves the machine.
cat >"$F/bin/curl" <<EOF
#!/bin/bash
while [ \$# -gt 0 ]; do case "\$1" in -o) out=\$2; shift 2;; -*) shift;; *) url=\$1; shift;; esac; done
echo "\$url" >> "$F/state/curl-calls"; cp "$F/protoc.zip" "\$out"
EOF
mkdir -p "$F/zip/bin" "$F/zip/include"
printf '#!/bin/bash\necho "libprotoc 36.2"\n' >"$F/zip/bin/protoc"
chmod +x "$F/zip/bin/protoc"
(cd "$F/zip" && zip -qr "$F/protoc.zip" bin include)
HPC_PROTOC_SHA256=$(sha256sum "$F/protoc.zip" | awk '{ print $1 }')
export HPC_PROTOC_VERSION=36.2 HPC_PROTOC_SHA256
# GNU stat -c %Y on top of BSD stat, for the Mac.
cat >"$F/bin/stat" <<'EOF'
#!/bin/bash
if [ "$1" = -c ] && [ "$2" = %Y ]; then exec /usr/bin/stat -f %m "$3" 2>/dev/null || exec /usr/bin/stat -c %Y "$3"; fi; exec /usr/bin/stat "$@"
EOF
command -v nproc >/dev/null || printf '#!/bin/bash\nsysctl -n hw.ncpu\n' >"$F/bin/nproc"
chmod +x "$F/ssh-local" "$F"/bin/*
export HPC_SSH="$F/ssh-local" HPC_REMOTE_ROOT="$F/remote" HPC_LOCAL_RUNS="$F/runs"

# A control patch that appends a marker line to README.md.
{ cat README.md; echo HPCTEST; } >"$F/new.md"
git diff --no-index -- README.md "$F/new.md" | sed "s|b$F/new.md|b/README.md|" >"$F/plan/ctl/break.patch"

# A control patch that adds a file in a new folder (its own paths must not read as DIRTY).
printf 'diff --git a/newdir/x.txt b/newdir/x.txt\nnew file mode 100644\n--- /dev/null\n+++ b/newdir/x.txt\n@@ -0,0 +1 @@\n+HPCTEST\n' >"$F/plan/ctl/newdir.patch"
ok=0 bad=0
check() { # check NAME WANT_EXIT WANT_TEXT COMMAND...
    local name=$1 want=$2 text=$3
    shift 3
    "$@" >"$F/out" 2>&1
    local got=$?
    if [ "$got" = "$want" ] && grep -q -- "$text" "$F/out"; then
        ok=$((ok + 1)); echo "ok    $name"
    else
        bad=$((bad + 1)); echo "FAIL  $name (exit $got, want $want, text '$text')"; sed 's/^/      /' "$F/out"
    fi
}
sub="$HERE/hpc-submit" fetch="$HERE/hpc-fetch" status="$HERE/hpc-status"

# A test command that passes on the commit and fails (test x::guard) with the control patch.
GUARD='if grep -q HPCTEST README.md; then echo "test x::guard ... FAILED"; echo "test result: FAILED. 2 passed; 1 failed; 0 ignored"; exit 101; else echo "test result: ok. 3 passed; 0 failed; 0 ignored"; fi'
BUILDERR='if grep -q HPCTEST README.md; then echo "error[E0308]"; exit 101; else echo "test result: ok. 1 passed; 0 failed; 0 ignored"; fi'
ALWAYS='echo "test result: ok. 3 passed; 0 failed; 0 ignored"'
NEWDIR='if [ -e newdir/x.txt ]; then echo "test n::d ... FAILED"; echo "test result: FAILED. 0 passed; 1 failed; 0 ignored"; exit 101; else echo "test result: ok. 1 passed; 0 failed; 0 ignored"; fi'
REDBASE='echo "test y::old ... FAILED"; echo "test result: FAILED. 0 passed; 1 failed; 0 ignored"; exit 101'
cat >"$F/plan/mixed.plan" <<EOF
pass unit echo "test result: ok. 3 passed; 0 failed; 1 ignored"; echo "test result: ok. 2 passed; 0 failed; 0 ignored"
pass base $GUARD
fail:ctl/break.patch caught $GUARD
fail:ctl/break.patch@x::guard named $GUARD
fail:ctl/break.patch@other wrongtest $GUARD
pass base2 $ALWAYS
fail:ctl/break.patch missed $ALWAYS
pass base3 $BUILDERR
fail:ctl/break.patch invalid $BUILDERR
pass base4 $NEWDIR
fail:ctl/newdir.patch newdir $NEWDIR
pass redbase $REDBASE
fail:ctl/break.patch nobase $REDBASE
pass nothing echo "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out"
pass build echo compiled
pass mess touch stray.txt; echo junk >> README.md; git -c user.email=t@t -c user.name=t commit -q --allow-empty -m x
pass stray touch stray2.txt
pass move git -c user.email=t@t -c user.name=t commit -q --allow-empty -m only-head
pass restored ! grep -q HPCTEST README.md && [ ! -e stray.txt ] && ! grep -q junk README.md && [ "\$(git rev-parse HEAD)" = "\$SHA" ]
any probe exit 3
any dirtyprobe touch stray3.txt; exit 3
EOF
touch "$F/state/no-toolchain"
check "submit runs, installs the pinned toolchain" 0 "submitted: job" "$sub" "$F/plan/mixed.plan"
grep -q "installing Rust 1.96.0" "$F/out" && { ok=$((ok + 1)); echo "ok    toolchain install logged"; } || { bad=$((bad + 1)); echo "FAIL  toolchain install not logged"; }
grep -q "installing protoc 36.2" "$F/out" && [ -x "$F/remote/tools/protoc/bin/protoc" ] && { ok=$((ok + 1)); echo "ok    protoc installed"; } || { bad=$((bad + 1)); echo "FAIL  protoc not installed"; }
check "fetch: control caught"     0 "caught  *fail  *101 .* ok  *x::guard$" "$fetch"
check "fetch: named test caught"  0 "named  *fail  *101 .* ok  *x::guard$" "$fetch"
check "fetch: other test failed"  0 "wrongtest  *fail  *101 .* WRONG-TEST " "$fetch"
check "fetch: control missed"     0 "missed  *fail  *0 .* MISSED " "$fetch"
check "fetch: build error INVALID" 0 "invalid  *fail  *101 .* INVALID " "$fetch"
check "fetch: patch's new folder is not DIRTY" 0 "newdir  *fail  *101 .* ok  *n::d$" "$fetch"
check "fetch: red baseline"       0 "nobase  *fail  *101 .* NO-BASELINE " "$fetch"
check "fetch: zero tests"         0 "nothing  *pass  *0 .* NO-TESTS " "$fetch"
check "fetch: no libtest is ok"   0 "build  *pass  *0 .* ok " "$fetch"
check "fetch: moved HEAD flagged" 0 "mess .* ok+DIRTY " "$fetch"
check "fetch: stray file flagged" 0 "stray .* ok+DIRTY " "$fetch"
check "fetch: HEAD-only move flagged" 0 "move .* ok+DIRTY " "$fetch"
check "fetch: commit restored"    0 "restored .* ok " "$fetch"
check "fetch: counts summed"      0 "unit  *pass  *0  *[0-9]*  *5  *0  *1  *ok " "$fetch"
check "fetch: overall"            0 "NOT AS EXPECTED" "$fetch"
check "fetch: dirty record-only step" 0 "dirtyprobe .* recorded(exit 3)+DIRTY " "$fetch"
check "fetch: odd record noted"   0 "NOTE: record-only steps that exited non-zero: probe" "$fetch"
printf 'pass a true\nfail:ctl/break.patch c echo other\n' >"$F/plan/nobase.plan"
check "control without baseline refused" 1 "has no baseline" "$sub" "$F/plan/nobase.plan" --dry-run
[ ! -d "$F/remote/run.lock" ] && { ok=$((ok + 1)); echo "ok    lock released"; } || { bad=$((bad + 1)); echo "FAIL  lock left behind"; }

printf 'pass a %s\nfail:ctl/break.patch@guard c %s\n' "$GUARD" "$GUARD" >"$F/plan/good.plan"
check "good plan" 0 "submitted" "$sub" "$F/plan/good.plan"
check "good plan: all as expected" 0 "ALL STEPS AS EXPECTED" "$fetch"
good=$(basename "$(dirname "$(ls -1t "$F/runs"/*/jobid | head -n 1)")")
mv "$F/remote/runs/$good/out/overall" "$F/remote/runs/$good/out/overall.kept" # killed after the last row
check "missing summary reads INCOMPLETE" 0 "INCOMPLETE: 2 of 2" "$fetch" "$good"

printf 'pass one true\npass die kill -9 $PPID\npass never true\n' >"$F/plan/die.plan"
printf 'pass a true\nany dirty touch stray4.txt; exit 3\n' >"$F/plan/dirtyany.plan"
check "dirty record-only step" 0 "submitted" "$sub" "$F/plan/dirtyany.plan"
check "dirty record-only step fails the run" 0 "NOT AS EXPECTED" "$fetch"
check "killed job" 0 "submitted" "$sub" "$F/plan/die.plan"
check "killed job reads INCOMPLETE" 0 "INCOMPLETE: 1 of 3" "$fetch"
killed=$(basename "$(dirname "$(ls -1t "$F/runs"/*/jobid | head -n 1)")")
echo 0 >"$F/remote/runs/$killed/out/overall" # as if the summary were written but rows were lost
check "missing rows read INCOMPLETE" 0 "INCOMPLETE: 1 of 3" "$fetch" "$killed"

calls=$(wc -l <"$F/state/curl-calls")
check "protoc present: no second download" 0 "prepare: protoc 36.2$" "$sub" --cmd true
[ "$(wc -l <"$F/state/curl-calls")" = "$calls" ] && { ok=$((ok + 1)); echo "ok    protoc not downloaded again"; } || { bad=$((bad + 1)); echo "FAIL  protoc downloaded again"; }
printf '#!/bin/bash\necho "libprotoc 3.12.4"\n' >"$F/remote/tools/protoc/bin/protoc"
check "old protoc replaced" 0 "installing protoc 36.2" "$sub" --cmd true
"$F/remote/tools/protoc/bin/protoc" --version | grep -q "36.2" && { ok=$((ok + 1)); echo "ok    old protoc now 36.2"; } || { bad=$((bad + 1)); echo "FAIL  old protoc kept"; }
rm -rf "${F:?}/remote/tools/protoc"
HPC_PROTOC_SHA256=0000000000000000000000000000000000000000000000000000000000000000 \
    check "protoc zip with a wrong digest refused" 1 "prepare failed" "$sub" --cmd true
grep -q "digest is .* expected 0000" "$F/out" && [ ! -e "$F/remote/tools/protoc" ] &&
    { ok=$((ok + 1)); echo "ok    wrong digest: nothing installed"; } || { bad=$((bad + 1)); echo "FAIL  wrong digest: protoc installed or not reported"; }
[ ! -d "$F/remote/run.lock" ] && { ok=$((ok + 1)); echo "ok    lock released after the digest refusal"; } || { bad=$((bad + 1)); echo "FAIL  lock left after the digest refusal"; }
cp "$F/protoc.zip" "$F/protoc.good.zip"
printf '#!/bin/bash\necho "libprotoc 9.9"\n' >"$F/zip/bin/protoc"
rm -f "$F/protoc.zip" && (cd "$F/zip" && zip -qr "$F/protoc.zip" bin include)
HPC_PROTOC_SHA256=$(sha256sum "$F/protoc.zip" | awk '{ print $1 }') \
    check "zip whose protoc reports another version refused" 1 "prepare failed" "$sub" --cmd true
grep -q "does not report libprotoc 36.2" "$F/out" && [ ! -e "$F/remote/tools/protoc" ] &&
    { ok=$((ok + 1)); echo "ok    wrong version: nothing installed"; } || { bad=$((bad + 1)); echo "FAIL  wrong version: protoc installed or not reported"; }
mv "$F/protoc.good.zip" "$F/protoc.zip"
check "protoc reinstalled" 0 "installing protoc 36.2" "$sub" --cmd true

touch "$F/state/wrong-rustc"
check "wrong toolchain refused" 1 "prepare failed" "$sub" --cmd true
rm -f "$F/state/wrong-rustc"
# The job itself refuses a wrong toolchain before any step (prepare's check bypassed).
mkdir -p "$F/bin2"; printf '#!/bin/bash\necho "rustc 1.100.0 (fake)"\n' >"$F/bin2/rustc"; chmod +x "$F/bin2/rustc"
cp "$F/home/.cargo/env" "$F/env.kept"
printf '%s\n' "[ -n \"\${SLURM_JOB_ID:-}\" ] && export PATH=\"$F/bin2:$F/bin:\$PATH\" || export PATH=\"$F/bin:\$PATH\"" >"$F/home/.cargo/env"
check "job refuses wrong toolchain" 0 "submitted" "$sub" --cmd true
check "job refused before any step reads INCOMPLETE" 0 "INCOMPLETE: 0 of 1" "$fetch"
nostart=$(basename "$(dirname "$(ls -1t "$F/runs"/*/jobid | head -n 1)")")
rm -rf "${F:?}/remote/runs/$nostart/out" # as if apptainer itself had failed
check "never-started plan reads INCOMPLETE" 0 "INCOMPLETE: the plan never started" "$fetch" "$nostart"
cp "$F/env.kept" "$F/home/.cargo/env"

echo "31000 other RUNNING" >"$F/state/queue"
n0=$(ls "$F/runs" | wc -l)
check "quota in use: refused" 3 "quota is in use" "$sub" --cmd true
[ "$(ls "$F/runs" | wc -l)" = "$n0" ] && { ok=$((ok + 1)); echo "ok    refused submit leaves no run folder"; } || { bad=$((bad + 1)); echo "FAIL  refused submit left a run folder"; }
rm -f "$F/state/queue"

touch "$F/state/offline"
check "no connection: sign-in hint" 1 "ssh -fN" "$sub" --cmd true
rm -f "$F/state/offline"

touch "$F/state/hold"
check "held job submitted" 0 "submitted" "$sub" --cmd true
check "status: quota in use" 0 "quota: IN USE" "$status"
check "fetch refused while pending" 1 "is PENDING" "$fetch"
check "second submit refused" 3 "quota is in use" "$sub" --cmd true
rm -f "$F/state/hold" "$F/state/queue"
check "stale lock (job gone) removed" 0 "removed stale lock" "$sub" --cmd true
mkdir "$F/remote/run.lock" && echo other >"$F/remote/run.lock/owner" && echo "999999 $(hostname)" >"$F/remote/run.lock/pid"
check "stale lock (preparer dead) removed" 0 "removed stale lock" "$sub" --cmd true
mkdir "$F/remote/run.lock" && echo other >"$F/remote/run.lock/owner" && echo "$$ $(hostname)" >"$F/remote/run.lock/pid"
check "live preparer's lock refused" 1 "being prepared (pid" "$sub" --cmd true
rm -rf "$F/remote/run.lock"
touch "$F/state/drop"
check "connection lost after sbatch: job recovered" 0 "recorded it" "$sub" --cmd true
rm -f "$F/state/drop"
check "recovered job fetches" 0 "ALL STEPS AS EXPECTED" "$fetch"

mkdir "$F/remote/run.lock" && echo other >"$F/remote/run.lock/owner"
check "fresh lock without a job refused" 1 "being prepared" "$sub" --cmd true
rm -rf "$F/remote/run.lock"

"$sub" --dry-run --cmd true >"$F/out" 2>&1
dry=$(sed -n 's/^dry run: run folder is \(.*\) (nothing sent)\.$/\1/p' "$F/out")
d="$F/remote/runs/mismatch"; mkdir -p "$d"; cp -R "$dry/upload/." "$d/"
sed -i.bak 's/^SHA=.*/SHA=0000000000000000000000000000000000000000/; s/^RUNID=.*/RUNID=mismatch/' "$d/meta.env"
check "bundle is not the expected commit" 4 "expected 0000" "$F/ssh-local" h "bash $d/bin/prepare.sh $d"
d2="$F/remote/runs/quota"; mkdir -p "$d2"; cp -R "$dry/upload/." "$d2/"
echo "31000 other RUNNING" >"$F/state/queue"
check "cluster side refuses on quota too" 3 "quota is in use" "$F/ssh-local" h "bash $d2/bin/prepare.sh $d2"
[ ! -e "$d2" ] && { ok=$((ok + 1)); echo "ok    refused prepare removes its upload"; } || { bad=$((bad + 1)); echo "FAIL  refused prepare left its upload"; }
d3="$F/remote/runs/hangup"; mkdir -p "$d3"; cp -R "$dry/upload/." "$d3/"
sed -i.bak 's/^RUNID=.*/RUNID=hangup/' "$d3/meta.env"
echo "31001 konnect-hangup PENDING" >"$F/state/queue" # its job exists: prepare must not delete what it needs
check "failed prepare with a queued job" 3 "quota is in use" "$F/ssh-local" h "bash $d3/bin/prepare.sh $d3"
[ -e "$d3/meta.env" ] && { ok=$((ok + 1)); echo "ok    a queued job keeps its run folder"; } || { bad=$((bad + 1)); echo "FAIL  a queued job lost its run folder"; }
rm -f "$F/state/queue"

check "limit: 81 CPUs" 1 "1..80" "$sub" --cpus 81 --cmd true
check "limit: over 4 h" 1 "at most 4:00:00" "$sub" --time 4:00:01 --cmd true
check "limit: bad label" 1 "label" "$sub" --label 'a;b' --cmd true
check "limit: --mem 0" 1 "whole node" "$sub" --mem 0 --cmd true
printf 'fail:nope.patch c true\n' >"$F/plan/nope.plan"
check "plan: missing patch" 1 "control patch not found" "$sub" "$F/plan/nope.plan" --dry-run
printf 'maybe c true\n' >"$F/plan/expect.plan"
check "plan: bad EXPECT" 1 "EXPECT must be" "$sub" "$F/plan/expect.plan" --dry-run

echo "selftest: $ok passed, $bad failed"
[ "$bad" = 0 ]
