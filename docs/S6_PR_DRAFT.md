# S6 proposed PR text

Status (2026-11-10): the owner approved publication and merge of this exact S6 work. The title and body below are published unchanged in [PR #62](https://github.com/yuvaltal912-sys/Konnect/pull/62). This file is retained as the publication record; do not create another PR from it. Read the PR for its current merge state.

## Exact title

```text
feat(routing): keep pour-net class widths and apply measured defaults
```

## Exact body

Body SHA-256 (UTF-8, including its final newline): `67f89c690f06997ce6a0c71209cf5290064e2a435d0792cf242ba3d6c57e2a05`. The markers delimit the body and are not part of it.

<!-- BEGIN PR BODY -->
## Summary

Implement the first half of creation row S6. Current-bearing nets that own copper pours retain their effective class widths, and `route_specctra_dsn` uses S5's measured Freerouting preset.

## Approach

Reuse the existing zone classifier and net resolution. Record the unchanged width and reason in `intent_widths`, including when pours are omitted. Other current-bearing nets still widen. Reuse the existing recursive settings readback to refuse routing if any preset value is missing or different.

The preset uses 20 passes, optimizer enabled, via cost 10, direction-cost scalar 1.0, consecutive-failure limits of 1,000,000, and a 0.25% improvement threshold. The job API receives `0.25`; the equivalent CLI receives `0.0025`.

## Branch and dependencies

Based on `main` at `7e9a174`, after merged S5 PR #60 and handoff PR #61. This PR owns only S6's pour-width exemption and measured preset. Per-zone pour policy, planes and stitching remain deferred.

## Compatibility and safety

No new public argument. Existing `max_passes` and `optimizer_enabled` overrides remain supported; the changed defaults are documented in `API_MIGRATIONS.md`. No-intent and explicit width-opt-out exports remain unchanged. Pour ownership does not certify current capacity.

| Behavior | Contract and evidence |
|---|---|
| Inputs and defaults | Two- and four-layer fixtures; preset and override tests |
| Invalid inputs | Missing widths still refuse export; missing/mismatched settings refuse routing |
| Target and source | Existing project rules, actual zone ownership, exported manifest |
| Preserved state | 18 demo DSNs byte-identical; original source hashes unchanged |
| Failure and recovery | Readback precedes routing; existing new-output-only and process-cleanup behavior retained |
| Applied work | Scratch-board scoring only; live GUI SES application remains unverified |

## Validation

- Green macOS gate: formatting; 4,201 workspace tests; 9 doc tests; 95 live simulation tests; Clippy with warnings denied; license check; unchanged documentation counts; release build.
- The documented IBIS cache flake occurred once; its isolated rerun and the full 95-test rerun passed without a code change.
- Seven new regression tests and nine caught source-mutation negative controls.
- Three product-MCP and three CLI runs: all six complete after KiCad refill and pass the signal-length criterion; all four median copper comparisons pass the declared parity band. All 12 product settings and actual per-layer routing parameters verified.
- Independent diff review found no actionable defects. Installed Konnect SHA-256 remains `3de188bb775ab4ce…`. Private evidence stays outside git; the creation handoff records commands and results.

Known S5 limitations remain: all six runs still fail the thermal-relief and power-width quality gates. Actual owner currents, pour capacity, another reference board, live GUI application, Windows and Linux are unverified. This PR does not claim electrical DRC readiness.
<!-- END PR BODY -->
