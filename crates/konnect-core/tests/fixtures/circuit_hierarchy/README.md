# Hierarchy-joined circuit fixtures

Generated on 2026-11-10 through the fork's creation tools, using KiCad's stock
`Timer:NE555D`, `Device:C` and `Device:R` symbols. These are synthetic designs,
not a private project. The serving fork was the retained baseline executable
built from `f246e48dab3994de9a0a6fa97176ab8238ce1db9`; its executable path and
embedded commit were read back before generation.

The fork created the base fixture and derived the repeated-child variant,
placed real library symbols, wired labels,
created the hierarchical SIGNAL port and annotated repeated instances. KiCad
10.0.8 then ran `sch upgrade --force` on each sheet and `sch export netlist
--format kicadsexpr` on each root, in isolated scratch settings under the
supervisor's OS write-denial guard. No raw schematic text was used to construct
the native fixtures. The committed netlists have only the absolute root source
path replaced by `cross_sheet.kicad_sch`; their connectivity is unchanged.

| Variant | Files | Sheet uses | Fitted parts | Physical pins |
|---|---:|---:|---:|---:|
| `two_sheet` | 2 | 2 | 4 | 14 |
| `repeated_child` | 2 | 3 | 8 | 22 |
| `local_power` | 2 | 3 | 8 | 22 |

In `two_sheet`, the root IC and child capacitor share global supply and ground.
The IC output reaches a child resistor through the explicit SIGNAL port.
Another resistor uses a local label, and five IC pins are separate singleton
nets. In `repeated_child`, the same saved child file is instantiated twice:
global supply/ground are shared, the two parent SIGNAL connections remain
separate, and each use retains its own local resistor net and designators.

`expected_partition.json` records the independently specified intended pin
sets using the saved instance identities. Generation compared the complete
pin universe and partition with KiCad's export. The Rust regression test
compares the joined model with the saved export, including singleton nets.

`local_power` is derived from `repeated_child` using a test-only helper built
against the repository's typed schematic model and revision-checked
S-expression transaction writer. It replaces the child capacitor's global
supply label with a genuine local-power symbol, preserving the full native
`IBF_supply:+7V` definition from KiCad 10.0.1's public QA fixture
`qa/data/eeschema/issue22938/Spannungsversorgung.kicad_sch`. Its original
`(power local)` definition is retained as `native_local_power_definition.sexp`;
the placed Value is overridden to `+3V3`, with fresh UUIDs and the two proper
saved instance records. The source definition is fixture data, not ported tool
implementation. One child-sheet upgrade and one root export by KiCad 10.0.8
confirm all 22 physical pins and three separate same-name supply nets. The
read-only oracle summary is retained beside the fixture.

These fixtures verify connectivity, not ERC cleanliness or physical placement.
The generated blank PCB and project/settings files are intentionally omitted.
Raw runtime/settings logs and absolute source paths remain in ignored evidence.
