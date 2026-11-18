"""Writes each net's effective routing rules, as KiCad resolves them, for the placement bench.

Run it with KiCad's own Python (it has the `pcbnew` module), on a scratch copy of a
board, with a scratch `KICAD_CONFIG_HOME` so KiCad never touches the real settings:

    KICAD_CONFIG_HOME=<scratch folder> \
    /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 -I \
        xtask/placement-bench/net_rules.py <board.kicad_pcb> <rules.json>

`rules.json` maps a net name to `class_name`, `constituents`, `track_width_mm`,
`clearance_mm`, `via_diameter_mm` and `via_drill_mm`. `place_route::rules_from_json`
reads it, and `export_specctra_dsn` would have read the same values from a live editor.

It prints counts only, never a net or a class name: the nets and the number of nets by
(clearance, track width) in mm.
"""

import collections
import json
import sys

import pcbnew

if len(sys.argv) != 3:
    sys.exit("usage: net_rules.py <board.kicad_pcb> <rules.json>")

b = pcbnew.LoadBoard(sys.argv[1])
mm = lambda nm: round(nm / 1e6, 6)
settings = b.GetDesignSettings().m_NetSettings
out, per = {}, collections.Counter()
for code in range(1, b.GetNetCount()):
    net = b.FindNet(code)
    if not net or not net.GetNetname():
        continue
    nc = settings.GetEffectiveNetClass(net.GetNetname())
    name = nc.GetName()
    out[net.GetNetname()] = {
        "class_name": name,
        "constituents": [name],
        "track_width_mm": mm(nc.GetTrackWidth()),
        "clearance_mm": mm(nc.GetClearance()),
        "via_diameter_mm": mm(nc.GetViaDiameter()),
        "via_drill_mm": mm(nc.GetViaDrill()),
    }
    per[(mm(nc.GetClearance()), mm(nc.GetTrackWidth()))] += 1
json.dump(out, open(sys.argv[2], "w"), ensure_ascii=False)
print(
    json.dumps(
        {
            "nets": len(out),
            "by_clearance_width": {f"{c}/{w}": n for (c, w), n in sorted(per.items())},
        }
    )
)
