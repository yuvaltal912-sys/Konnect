# S11 implementation plan

> Execute in this S11 worktree using the reviewed design and test-first steps.
> Local evidence/progress: `target/creation-research/s11/implementation/`.

**Goal:** annotate supported zone/pad and zone/via DRC candidates without
changing DRC or claiming an unproven engine cause.

**Architecture:** a private child module of `drc.rs` owns snapshot capture,
structural identity/net/layer lookup, strict report recognition and bounded
diagnostics. The existing DRC flow supplies its primary and optional private
comparison reports; no extra process or board write is added.

**Tech stack:** pinned Rust, existing `konnect-sexp`, `serde_json`, `tokio`.
**Spec:** [`S11_DESIGN.md`](S11_DESIGN.md).

## Constraints

- Published claim #64 at `a423ec8`; no later push/text approval yet.
- Source scope: `tools/zone_fill_defects.rs`, its tests, narrow `tools/drc.rs`.
- No S7/cross-sheet/shared geometry/CLI/registry edits or new dependency.
- No native or broad runs until the supervisor releases their slot. Pure Rust
  runs also await a lane. Use own target and scratch KiCad configuration.
- Keep original reports intact. Preserve null/unavailable versus checked zero.
- No private board geometry/net identities in tracked fixtures or documents.

## Review focus

- A report from CLI in-memory refill must identify that provenance.
- Wrong/stale snapshots must not lend authority to a correct-looking UUID.
- Tiny nonzero clearances must not become numeric zero by tolerant parsing.
- Reordered report items or changed prose must not create a new pair.
- Partial evidence and truncated examples must not erase valid candidates.

## Task 1: primary observations and recognition

Files: new classifier/tests; `drc.rs`'s primary-report capture and diagnostic
attachment. Interface: `Snapshot::capture(&Path) -> Result<Snapshot, String>`,
`Snapshot::still_matches(&Path) -> bool`; `Inspection::new(&DrcReport,
Result<&Snapshot, &str>, FillSource) -> Inspection`;
`diagnostic(&Inspection, Option<&Inspection>, Option<&str>) -> Value`.

- [ ] Add a served regression using the existing `zone_fill_tests::Scene`:
  reported zero from a structurally resolved different-net zone/pad yields
  `suspected`, one primary entry, one pair, null refill attribution, no extra
  CLI call and unchanged original report/source. Run it and retain expected
  failure for absent diagnostic before writing the implementation.
- [ ] Add table-driven pure tests for the spec's identity/net/layer, rule,
  reported-zero, short, ambiguity and output-budget matrix. Expected values
  are literals; fixtures are synthetic Rust strings, not private board copies.
- [ ] Implement the new module and primary integration. Snapshot capture is
  best-effort and must never replace an otherwise valid DRC result with an
  error. A source change produces incomplete observation evidence.
- [ ] Run targeted tests until green; retain the red/green command logs.

## Task 2: private-refill comparison and attribution

Files: same module/tests and `zone_fill_check` in `drc.rs`. `Inspection`
retains unique candidate pair identities and a source-normalized design/fill
fingerprint; public strings alone are bounded, internal identities stay exact.
Comparison consumes primary and scratch snapshots, with before/after original
source stability checks around the existing private refill.

- [ ] Tests: absent-then-zero, nonzero-then-zero, existing-pair rule/severity/
  position/order/multiplicity changes, changed pad/net/rules, unchanged fill,
  unsupported conversion, failed comparison and duplicate UUIDs. Run red.
- [ ] Capture refilled scratch text/sidecars before its temporary directory
  drops. Normalize only explicitly supported numeric formatting/order and
  serialization metadata; retain every design/rule/unknown field. Require
  changed implicated fill for a newly qualifying pair. Source instability or
  incomplete inspection removes attribution without erasing valid observations.
- [ ] Attach on all zone-fill return paths. Preserve prior fill-state semantics
  and private-refill call count. Test both DRC surfaces and baseline propagation.
- [ ] Run green targeted tests and existing zone-fill regressions.

## Task 3: acceptance, guard evidence and release

- [ ] Replay retained private report/snapshot pairs through the real detector:
  four matching entries/two pairs versus zero; no causal comparison of those
  differently routed boards. Use an opt-in ignored test with input paths in
  environment variables and no private data in git.
- [ ] Run six serial native scratch refill comparisons from a frozen input
  with scratch configuration, three per iterative setting. Retain starting
  fill state, all reports/snapshots/hashes and honest reproduction outcome.
- [ ] Run focused llvm-cov and targeted cargo mutants in a disposable checkout,
  plus deliberate per-guard source breaks. Record every caught/unviable/timed-
  out/equivalent/surviving case; restore and prove the candidate diff unchanged.
- [ ] Update troubleshooting, API migration and handoff with actual results.
- [ ] Fresh independent implementation diff review; fix verified findings with
  regressions, run required gate, update Graphify, noreply commit and post-commit
  gate/release stdio dogfood. Claim no platform/GUI check not actually run.
- [ ] Present exact final PR update text and implementation push for approval.

## Execution notes

The supervisor already authorized execution of the reviewed design. Local
commits follow the project gate/review sequence rather than per-task pushes.
The implementation ledger records test commands, results, decisions and the
next concrete step; completed work is not restarted after a context handoff.
