# Specctra per-zone pour policy fixture

`specctra_pours_policy.kicad_pcb` is `specctra_rounded_board.kicad_pcb` with four
copper pours and one rule area. The tests of the per-zone `pours` policy
(`specctra_pours_tests.rs`) use it:

| Zone | Net | Layers | UUID |
|---|---|---|---|
| `GND_BOTTOM` (the rounded board's pour, now named) | GND | B.Cu | `44444444-4444-4444-8444-444444444441` |
| `GND_TOP` | GND | F.Cu | `55555555-5555-4555-8555-555555555501` |
| `VCC_REGION` (priority 1) | VCC | F.Cu | `55555555-5555-4555-8555-555555555502` |
| `GND_BOTH` | GND | F.Cu and B.Cu | `55555555-5555-4555-8555-555555555503` |
| `no_route_keepout` (rule area: no tracks, no vias) | none | F.Cu | `44444444-4444-4444-8444-444444444442` |

That makes five pour layers. Selecting by name, by net and layer, or by UUID
and layer gives a different set of planes each time, and `GND_BOTH` is the zone
that only a layer selector can split.

## Provenance

Written by a script from `specctra_rounded_board.kicad_pcb` on 2026-11-13: it
names the existing B.Cu pour and adds the three new zones in KiCad 10's zone
syntax. The zones have outlines and no saved fill. The exporter uses the
outline, never the fill, so the file is not refilled. It has not been re-saved
by KiCad.
