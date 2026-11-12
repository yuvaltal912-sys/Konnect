# S11 claim publication text

Approved and published 2026-11-10 as draft PR #64, claim `a423ec8`. This text is for the initial
claim-only draft PR. Implementation and final publication text are still owed.

The body below is kept byte for byte as the historical publication record
(SHA-256 `8af4bf53ae9df21b7f84893a812c837ba40db19ca6d698825325f96cb29a6a07`). Its sentence
"KiCad's reported zero can be a rounded distance." is superseded and must not
be repeated: inspected KiCad source formats tiny nonzero measurements
scientifically. The corrected wording, for all later text, is:

The diagnostic reports KiCad's formatted measurement; it does not independently prove physical overlap or an engine bug.

Push destination: `fork`, branch `codex/creation-s11-zone-fill-defects`.
Base: `main`. Local claim commit subject:
`docs: claim creation S11 zone-fill defect detection`.

## Exact title

feat(creation): report suspected zone-fill defects (S11)

## Exact body

This draft claims creation row S11. It currently contains preparation only;
the detector is not implemented.

The planned read-only diagnostic identifies KiCad DRC reports of a copper
zone against another net's pad or via at reported zero clearance, or a
shorted pair. It separates observed candidates from findings newly reported
after a comparable private refill. Neither is treated as proof of an engine
bug, and all original DRC violations, severities and counts remain intact.

Scope: a new `tools/zone_fill_defects.rs` module and tests, narrow integration
in `tools/drc.rs`, and troubleshooting/API/handoff documentation. The result
will live at `zone_fill_check.suspected_fill_defects`. No new tool, extra
KiCad run, board repair or KiCad configuration change is planned. S7's planner
and the parallel cross-sheet review keep separate source ownership.

The macOS baseline passed at `f246e48`: 4,201 workspace tests, 9 doctests,
95 simulation regression tests, formatting, Clippy, license checks, current
documentation counts and the release build. No retries were needed; the
checked KiCad/Freerouting settings and installed Konnect binary are unchanged.

Still required before this PR is ready: detector tests and guard-break
negative controls; matched before/after evidence tests; retained-report replay
and bounded native scratch checks; scoped coverage and mutation checks;
independent diff review; final gates; and a release stdio check. Each command
and result will be recorded. Heavy runs remain serialized with the other
creation work.

KiCad's reported zero can be a rounded distance. Missing identities, unknown
net data, unsupported report syntax or an invalid comparison will be explicit
limits, not a clean result or a causal claim. Windows, Linux and live GUI
behavior remain outside this row's validation.
