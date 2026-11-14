"""Write the synthetic pour-routing fixture F1 for row S6's second half.

    /opt/homebrew/bin/python3.14 -I specctra_pours_routing.make.py <out-dir>

Writes <out-dir>/specctra_pours_routing.kicad_pcb and .kicad_pro: a 2-layer
50 x 40 mm board with twelve SMD 0805 resistors in a 4 x 3 grid. Pad 2 of
every resistor is GND; pad 1 pairs each resistor with the one diagonally
opposite (six signals that must cross the board). A GND pour covers each
side, so the signals cut both pours. Deterministic: UUIDs come from uuid5.
Plain Python, no KiCad.
"""
import json
import sys
import uuid
from pathlib import Path

NS = uuid.UUID('5f6b3d4e-0000-4000-8000-000000000f01')


def uid(*parts):
    return str(uuid.uuid5(NS, '/'.join(map(str, parts))))


HEADER = '''(kicad_pcb
\t(version 20260206)
\t(generator "pcbnew")
\t(generator_version "10.0")
\t(general
\t\t(thickness 1.6)
\t\t(legacy_teardrops no)
\t)
\t(paper "A4")
\t(layers
\t\t(0 "F.Cu" signal)
\t\t(2 "B.Cu" signal)
\t\t(9 "F.Adhes" user "F.Adhesive")
\t\t(11 "B.Adhes" user "B.Adhesive")
\t\t(13 "F.Paste" user)
\t\t(15 "B.Paste" user)
\t\t(5 "F.SilkS" user "F.Silkscreen")
\t\t(7 "B.SilkS" user "B.Silkscreen")
\t\t(1 "F.Mask" user)
\t\t(3 "B.Mask" user)
\t\t(25 "Edge.Cuts" user)
\t\t(27 "Margin" user)
\t\t(31 "F.CrtYd" user "F.Courtyard")
\t\t(29 "B.CrtYd" user "B.Courtyard")
\t)
\t(setup
\t\t(pad_to_mask_clearance 0)
\t\t(allow_soldermask_bridges_in_footprints no)
\t)
'''


def prop(name, value, at, layer, hidden, key):
    hide = '\n\t\t\t(hide yes)' if hidden else ''
    return f'''\t\t(property "{name}" "{value}"
\t\t\t(at {at})
\t\t\t(layer "{layer}"){hide}
\t\t\t(uuid "{uid(key, name)}")
\t\t\t(effects
\t\t\t\t(font
\t\t\t\t\t(size 1 1)
\t\t\t\t\t(thickness 0.15)
\t\t\t\t)
\t\t\t)
\t\t)
'''


def resistor(ref, x, y, rotation, net1, net2):
    pads = ''
    for number, dx, net in (('1', -0.95, net1), ('2', 0.95, net2)):
        pads += f'''\t\t(pad "{number}" smd roundrect
\t\t\t(at {dx} 0 {rotation})
\t\t\t(size 1 1.25)
\t\t\t(layers "F.Cu" "F.Mask" "F.Paste")
\t\t\t(roundrect_rratio 0.25)
\t\t\t(net "{net}")
\t\t\t(uuid "{uid(ref, 'pad', number)}")
\t\t)
'''
    return (f'''\t(footprint "Resistor_SMD:R_0805_2012Metric"
\t\t(layer "F.Cu")
\t\t(uuid "{uid(ref)}")
\t\t(at {x} {y} {rotation})
''' + prop('Reference', ref, f'0 -1.65 {rotation}', 'F.SilkS', False, ref)
            + prop('Value', '10k', f'0 1.65 {rotation}', 'F.Fab', False, ref)
            + prop('Datasheet', '', '0 0 0', 'F.Fab', True, ref)
            + prop('Description', '', '0 0 0', 'F.Fab', True, ref)
            + '\t\t(duplicate_pad_numbers_are_jumpers no)\n' + pads
            + '\t\t(embedded_fonts no)\n\t)\n')


def line(x1, y1, x2, y2, key):
    return f'''\t(gr_line
\t\t(start {x1} {y1})
\t\t(end {x2} {y2})
\t\t(stroke
\t\t\t(width 0.05)
\t\t\t(type default)
\t\t)
\t\t(layer "Edge.Cuts")
\t\t(uuid "{uid('edge', key)}")
\t)
'''


def zone(name, layer, x0, y0, x1, y1):
    return f'''\t(zone
\t\t(net "GND")
\t\t(layer "{layer}")
\t\t(uuid "{uid('zone', name)}")
\t\t(name "{name}")
\t\t(hatch edge 0.5)
\t\t(connect_pads
\t\t\t(clearance 0.3)
\t\t)
\t\t(min_thickness 0.2)
\t\t(filled_areas_thickness no)
\t\t(fill yes
\t\t\t(thermal_gap 0.3)
\t\t\t(thermal_bridge_width 0.3)
\t\t\t(island_removal_mode 1)
\t\t\t(island_area_min 10)
\t\t)
\t\t(polygon
\t\t\t(pts
\t\t\t\t(xy {x0} {y0}) (xy {x1} {y0}) (xy {x1} {y1}) (xy {x0} {y1})
\t\t\t)
\t\t)
\t)
'''


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    body = HEADER
    parts = []
    index = 0
    for row in range(3):
        for column in range(4):
            index += 1
            x = 108 + column * 11
            y = 108 + row * 12
            rotation = 0 if (row + column) % 2 == 0 else 90
            parts.append((f'R{index}', x, y, rotation))
    # Pad 1 of R(k) and R(13-k) share a signal: diagonally opposite parts.
    for ref, x, y, rotation in parts:
        k = int(ref[1:])
        signal = f'S{min(k, 13 - k)}'
        body += resistor(ref, x, y, rotation, signal, 'GND')
    corners = [(100, 100), (150, 100), (150, 140), (100, 140)]
    for i, (a, b) in enumerate(zip(corners, corners[1:] + corners[:1])):
        body += line(*a, *b, i)
    body += zone('GND_TOP', 'F.Cu', 101, 101, 149, 139)
    body += zone('GND_BOTTOM', 'B.Cu', 101, 101, 149, 139)
    body += '\t(embedded_fonts no)\n)\n'
    (out / 'specctra_pours_routing.kicad_pcb').write_text(body)
    project = {
        'board': {'design_settings': {'rules': {
            'min_clearance': 0.2, 'min_track_width': 0.2, 'min_copper_edge_clearance': 0.3,
            'min_hole_clearance': 0.25, 'min_hole_to_hole': 0.25, 'min_via_annular_width': 0.1,
            'min_via_diameter': 0.6, 'min_through_hole_diameter': 0.3}}},
        'net_settings': {'classes': [{'name': 'Default', 'clearance': 0.2, 'track_width': 0.25,
                                      'via_diameter': 0.6, 'via_drill': 0.3}]},
    }
    (out / 'specctra_pours_routing.kicad_pro').write_text(json.dumps(project, indent=2) + '\n')
    print('wrote', out)


if __name__ == '__main__':
    main()
