# S7 claim PR text

Status (2026-11-10): the exact claim title/body below were approved and published unchanged in [draft PR #63](https://github.com/yuvaltal912-sys/Konnect/pull/63) at `70605289d757ded6e56d26ec9cb1c2631b979149`. S7 was stopped on 2026-11-10 and resumed on 2026-11-12 under the three-PR creation delegation. This file preserves the historical claim text; the PR's current title and body are on GitHub, approved by the creation supervisor.

## Exact title

```text
feat(routing): plan stitching vias and corner cleanup without mutation
```

## Exact body

Body SHA-256 (UTF-8, final newline included): `e63b6b91e71e2fa5fc85565509e1675926f487aa255ea5a839dc784347091d70`. Markers are not part of the body.

<!-- BEGIN PR BODY -->
## Purpose

Claim creation row S7 for a read-only stitching-via and corner-cleanup planner. This draft currently contains the claim and implementation plan only; no planner code is implemented yet.

## Planned scope

Add `plan_route_refinement` to `pcb_routing` (18 to 19 tools). It will propose same-net stitching vias, perpendicular retermination of acute T-junctions, and 45-degree chamfers for right-angle pad-exit doglegs.

The planner will return a versioned, fingerprinted plan with `mutated: false`, exact object preimages, proposed geometry, checks, conflicts and blocked reasons. It will use conservative saved-rule bounds and check complete copper shapes, connectivity and cumulative proposal conflicts. Unsupported safety geometry or rules will remain explicit blockers.

Reuse the layout audit through narrow internal model/detector extraction. The AI compares the result with its own design using `audit_layout_quality` and selectively adopts proposals through existing create/modify tools. The planner will not apply, save, refill or autoroute a board. Adoption is not a single atomic operation.

## Ownership and dependencies

Based on merged S6 PR #62 at `f246e48dab3994de9a0a6fa97176ab8238ce1db9`. S7 owns the new `route_refine` module, tests and fixtures, narrow layout-audit reuse seams, routing registration/counts, API documentation and the PCB skill loop. Independent creation work is coordinated in separate worktrees.

S6's remaining plane policy, S9 staging, the S11 detector and cross-sheet review remain outside this PR.

## Baseline and planned validation

The macOS baseline passed formatting, 4,201 workspace tests, 9 doc tests, Clippy with warnings denied, licenses, unchanged documentation counts and the release build. The first live simulation run passed 94 of 95 tests and hit the documented IBIS missing-cache failure; the isolated retry and full 95-test retry passed without a code change. Protected settings and the installed Konnect hash remained unchanged.

Before this draft becomes ready: synthetic two-/four-layer fixtures; a caught source-mutation negative control for every guard; scoped coverage and targeted mutation testing; release-binary stdio checks; bounded scratch application/refill/DRC and audit measurements from retained routing outputs; independent implementation review; and the full final gate.

Acceptance will compare normalized violation identities as well as counts, preserve terminal connectivity, and retain the original corner, island and length targets. Existing thermal-relief and width-gate failures remain separately reported. No S7 behavioral result, live GUI adoption, current capacity, Windows or Linux validation is claimed at this stage.
<!-- END PR BODY -->
