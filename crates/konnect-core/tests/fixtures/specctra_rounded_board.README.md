# Specctra rounded-board fixture

`specctra_rounded_board.kicad_pcb` is the two-resistor Specctra fixture with the
three things the first routing profile used to refuse:

- a **rounded outline**: four straight sides and four 3 mm quarter arcs (`gr_line`,
  `gr_arc`) on a 50 x 40 mm board;
- **`roundrect` pads** (`roundrect_rratio 0.25`, 0.6 x 0.5 mm) on both resistors;
- **two zones**: a GND copper pour on `B.Cu`, and a rule area on `F.Cu` that forbids
  tracks and vias.

## Provenance

Derived from `specctra_two_resistors.kicad_pcb` with a script (pads to `roundrect`,
the four outline lines replaced by lines and arcs, the two zones added), then opened
and re-saved by KiCad 10.0.8 so the committed text is KiCad's own serialization:

```text
kicad-cli pcb upgrade --force specctra_rounded_board.kicad_pcb
```

KiCad kept both zones, reordered and re-oriented the outline pieces (the arcs run
start-to-end in the opposite sense to the lines beside them, which the exporter must
not care about) and rounded each arc's `mid` to five decimals.

`specctra_rounded_board.native-kicad-10.dsn` is KiCad 10.0.8's own export of this
board (`pcbnew.ExportSpecctraDSN`), with the output path in the root `(pcb ...)`
identifier normalized to `board.dsn`. It shows what KiCad itself writes for the same
board: the outline as chords (15 per quarter arc), the GND pour as
`(plane GND (polygon B.Cu ...))`, the rule area as `(keepout "" (polygon F.Cu ...))`,
and each rounded pad as a polygon. The tests compare Konnect's DSN with it on the
plane and keepout, not on the outline's vertices or the pad polygon, which differ by
design (Konnect's chords stay within 5 um; its rounded pad is the bounding rectangle).

`specctra_rounded_board.freerouting-2.3.0.ses` is the routes of a real Freerouting 2.3.0
run on the two-resistor board of `specctra_two_resistors_locked`, whose pads and
placement are the same as here, with its identifiers changed to the ones Konnect's DSN
for this board uses (`konnect_image_R1`, `konnect_via_0001`). It was **not** produced from
this board's DSN: Freerouting 2.x needs Java 21 and none was available when this was
written. It exercises the import planner on a board with zones; the ignored test
`freerouting_routes_the_rounded_board` runs the real engine on this board's DSN when
`FREEROUTING_JAR` and a Java 21 are present.
