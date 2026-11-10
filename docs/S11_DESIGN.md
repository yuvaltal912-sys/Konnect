# S11: suspected zone-fill defects

Proposed contract, 2026-11-10. Branch `codex/creation-s11-zone-fill-defects`,
base `f246e48dab3994de9a0a6fa97176ab8238ce1db9`. Preparation only: no product
code, published claim or publication approval yet. The unchanged-source baseline
has passed; see the S11 handoff and local baseline records.

## Purpose and boundary

When KiCad DRC reports a copper zone against another net's pad or via at
reported zero clearance, or reports that pair shorted, explain that the
finding is consistent with a suspected fill defect. Distinguish an observed
pattern from a finding newly reported after a controlled refill. Neither
statement proves a KiCad engine bug or establishes that routing is correct.
Preserve every original violation, severity, count and DRC verdict.

This is a read-only diagnostic inside the existing DRC flow. It adds no tool,
argument, registry entry, refill, save, repair, plotting action or configuration
edit. Existing caller-requested synchronization/refill behavior remains the
caller's choice. No upstream issue is posted.

The known KiCad 10.0.8 iterative-refill behavior motivates this detector;
version alone, zone overlap alone or differing priorities alone is not proof.
Documentation will say not to plot until DRC is clean, explain the manual
`ZoneFillIterativeRefill=0` workaround for the affected KiCad version, and
require refill plus DRC afterward. Konnect never changes that setting. Native
Python `pcbnew` filling does not establish that the configuration switch ran.

## Approaches

1. A description-only detector is small but cannot safely establish item
   types, net identity or refill attribution. Reject this as the sole method.
2. **Proposed:** resolve report items against the board snapshot, recognize a
   narrow reported-zero/short signature, and reuse the existing private-refill
   comparison for stronger attribution when its evidence is complete.
3. An independent polygon-overlap engine or automatic two-configuration retry
   could provide other evidence, but expands geometry, runtime and mutation
   scope. Defer it. The S11 diagnostic is not an independent geometric oracle.

## Result contract

Add `zone_fill_check.suspected_fill_defects`; the existing baseline
source-evidence propagation will carry the diagnostic at
`source_evidence.zone_fill.suspected_fill_defects`.

- `status`: `suspected`, `none_detected`, `incomplete` or `not_checked`.
  `none_detected` means no supported signature in the inspected reports; it
  never means the board is safe or all fill defects were excluded. A valid
  candidate takes precedence (`suspected`) even when coverage is partial.
  With no established candidate, unresolved evidence yields `incomplete`;
  `none_detected` requires complete inspection of the available reports.
  Coverage and attribution availability are separate from this status.
- Separate `primary_report` and `comparison_report` summaries. Each names whether
  it was inspected, its matching violation-entry count, distinct item-pair
  count and unresolved-candidate count. Unavailable reports use `null` counts,
  not a checked zero. Counts are computed before list truncation. Explicit
  provenance distinguishes saved fills, CLI in-memory refill, saved IPC refill
  and the private-copy comparison; CLI refill does not imply a saved fill.
- `newly_reported_after_refill`: count only with a complete, comparable pair;
  otherwise `null` with a reason. This is report attribution, not a claim that
  an exact geometric copper difference or engine root cause was proven.
- At most ten `findings`, with `truncated`, full counts, report phase, rule,
  item UUIDs/kinds, the reported position, evidence basis and attribution.
  All externally derived strings, including UUIDs and reasons, are bounded
  so ten pathological strings cannot defeat the existing response budget.
  Original DRC output remains untouched.
- One concise hint and explicit coverage/attribution reasons. An inspection
  failure must not hide DRC results or substitute a successful clean result.

Default `check_zone_fills` reuses its existing two reports. With
`refill_zones: true`, inspect the available report but do not invent a before
report or start a third check. With `check_zone_fills: false`, any available
report-only finding has unproven refill attribution. A failure of the private
refill may still leave a valid saved-report observation. Footprint zones are
outside the CLI refill comparison; unresolved or unsupported evidence is
disclosed. A fresh fill can still contain a suspected defect.

## Detection and attribution guards

1. Support only `clearance`, `hole_clearance` and `shorting_items` violations.
   Clearance requires a strictly recognized actual-value suffix reporting
   numeric zero in recognized units. Read the actual value, not the required
   clearance. Do not use an epsilon or guess translated/malformed prose.
   `shorting_items` is a structured rule key and needs no zero-value prose.
2. Resolve a unique zone and a unique pad or via from item UUIDs in the exact
   board snapshot. Do not infer kinds or nets from human descriptions, item
   order or the first item's position. Missing/duplicate/reassigned UUIDs,
   extra ambiguous items, rule areas and unsupported ownership are incomplete
   evidence. A track/pad pair is not a candidate.
3. Prove different assigned nets from unambiguous board net data, including
   supported legacy numeric and current named forms. Net zero, missing names,
   inconsistent declarations and unmapped numeric references do not prove a
   different net. Bind every observation's identity lookup to its unchanged
   source snapshot, not only the stronger refill-attribution count. Require
   compatible copper layers. Same-net pairs do not
   qualify. A hole-clearance candidate also needs a supported plated pad/via;
   a mechanical hole alone does not establish another net's copper.
4. Keep `reported_zero_clearance` distinct from physical overlap. KiCad formats
   the actual distance into prose; a small positive value can round to zero.
   Do not derive a geometric overlap point from the first item's position.
5. For `newly_reported_after_refill`, bind the initial report to an unchanged
   captured board/project/rules snapshot, capture the refilled scratch board
   before it is removed, and require unique matching identities, unchanged
   non-fill design data and an actual change in the implicated zone's saved
   fill. Comparison normalization may allow explicit serialization metadata
   and ordering differences, but not geometry, nets, priorities, rules or
   unknown fields. Unsupported KiCad conversions lose attribution safely.
6. Compare candidate identities by unordered UUID pair, recording rule and
   multiplicity separately, not descriptions or rounded coordinates. A pair
   already qualifying before refill is not newly introduced when its rule,
   severity, prose, position or multiplicity changes; a nonzero-before/zero-
   after change may qualify. Count the after-report entries only for pairs
   newly qualifying in a valid comparison.
   Do not reuse the existing general `drc::violation_key` for this purpose.
7. No cleanup, retry, suppression, severity downgrade or routing blame follows
   from a finding. The manual workaround remains a suggestion for inspection.

## Source scope and integration

- New `crates/konnect-core/src/tools/zone_fill_defects.rs` and
  `zone_fill_defects_tests.rs`: pure parsing, classification, conservative
  snapshot comparison, output shaping and synthetic tests.
- Narrow integration in `tools/drc.rs`: declare the new child module with
  `#[path]`, capture evidence at the existing DRC boundaries, and attach the
  diagnostic on all applicable return paths. Reuse the scratch-refill result;
  retain its board text before the temporary directory is dropped.
- `docs/TROUBLESHOOTING.md`, `docs/API_MIGRATIONS.md`, this design and the S11
  handoff entry. No registry or count change is expected. Shared documentation
  entries must preserve S7's changes during eventual integration.

No edit is planned for `tools/mod.rs`, `tools/cli.rs`, shared geometry, S7's
planner/tests/fixtures/skill, simulation or S6 runtime files. If an interface
change outside this scope becomes necessary, flag it to the supervisor before
editing. The newer Rust-validation rule in the main checkout's `AGENTS.md`
applies without copying its unrelated in-progress edit into this branch.

Source inspected: `drc.rs`'s `run`, `zone_fill_check`, `pair_zones` and
`violation_key`; `tools/cli.rs`'s `DrcViolation`, `ReportItem`,
`refill_drc_on_private_copy` and report parser; `konnect-sexp` UUID and net
helpers. The private-refill report currently lacks ownership enrichment;
resolve its objects locally in the new module without expanding `cli.rs`.
Graphify guided discovery but its DRC nodes are stale; current source governs.

## Verification and negative controls

The supervisor released the slot after S7. **S11's own baseline passed at
`f246e48`: 4,201 workspace tests, 9 doctests and 95 simulation regression tests,
plus every other gate step.** No retry was needed. Protected files were
unchanged. The historical S6 gate was not used as S11 baseline evidence.

The baseline ran from this worktree before implementation. Continue using
the pinned toolchain, Homebrew first on PATH,
`KICAD_DEMOS=/Users/yuvaltal/ClaudeCode/kicad-10.0.1/demos`, a dedicated
`CARGO_TARGET_DIR` outside the main checkout, and scratch `KICAD_CONFIG_HOME`.
Copy the global library tables into the scratch configuration when native
checks need them. Hash protected files before/after. The required gate's
simulation test is regression validation only; simulation code is out of scope.

1. Unit matrix: pad/via, reversed item order, legacy/current net syntax,
   reported zero versus 0.15 mm, nonzero required versus zero actual, tiny
   positive/rounded prose, same net, unassigned net, missing/duplicate UUID,
   rule area, track/pad, incompatible layer, malformed/translated text,
   unrelated rules, shorted pair, repeated entries, ten-item truncation and
   bounded strings. Every classification guard gets a deliberate source-break
   negative control that must make a meaningful test fail.
2. Matched-report matrix: absent then present, already present, cleared,
   positive then zero, reordered descriptions/items, changed pad/net/rules,
   unchanged fill, duplicate IDs, incomplete prior report and unsupported
   serialization changes. Zero false causal claims is the threshold: these
   are discrete evidence decisions, not an approximate geometry score.
3. Served integration: both `run_drc` and `get_drc_violations`, DRC baseline
   propagation, checks skipped, caller-requested refill, comparison failure,
   source change during the check and footprint zones. Assert unchanged
   original DRC counts/verdict, unchanged call count and source bytes.
4. Retained private-report replay: expected four matching violation entries
   in the faulty report, two distinct pairs; zero in the clean report. These
   are recognition tests only. Do not use two independently routed boards as
   a before/after oracle. No private geometry or identifying details enter git.
5. Native scratch checks: three serial controlled comparisons per iterative
   refill setting from the same frozen input, keeping raw reports, snapshots,
   versions and configuration hashes. Record whether the frozen input already
   contains the suspect fill: repeating that fill may produce four observed
   entries and zero newly qualifying entries. Six bounded refills check
   repeatability and geometry-sensitive behavior without a routing sweep.
   Record non-reproduction honestly;
   never force the expected four findings or claim repeatability from stored
   reports. Do not run Freerouting. Use existing public KiCad demo scratch
   checks to verify normal-result preservation; no GUI claim is implied.
6. Targeted `cargo llvm-cov` for the new classifier/integration evidence, with
   all decision branches exercised or their omissions explained. Targeted
   `cargo mutants` in a separate disposable checkout for detection and
   attribution guards, alongside explicit guard-break controls. Require zero
   unexplained surviving non-equivalent guard mutants; distinguish timeouts,
   unviable mutations and equivalence from caught mutants. Keep reports/builds
   outside the main checkout. Discover command support before selecting exact
   filters; record every command, result and skipped-check reason.
7. Independent diff review, fix verified findings, rerun the gate, update
   Graphify after code changes, noreply commit, then the required post-commit
   gate and release-binary stdio check of executable path/build commit and the
   new result. Recheck the installed binary SHA-256 prefix `3de188bb775ab4ce`.

## Uncertainty and next step

Native reproduction on this machine has not been attempted in S11. Report
syntax outside the supported recognizer is incomplete evidence; Windows,
Linux, GUI behavior and an upstream-confirmed root cause remain unverified.
No owner requirement is currently missing. These technical uncertainties
have bounded checks and explicit unknown outcomes.

The supervisor reviewed this contract with no remaining design blocker and
the baseline passed. Next: finish the local claim-only handoff commit; obtain
approval for the exact claim push and draft PR title/body; publish the claim;
only then implement. Final implementation/publication text will be reviewed
separately. All S11 heavy commands have exited and the slot is released.
