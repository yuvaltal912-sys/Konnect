"""Put a Freerouting result into a scratch copy of a board, and read a board's
routing back as plain numbers, for `cargo xtask freerouting-bench --score`.

Run it with KiCad's own Python, which has the `pcbnew` module, with `-I -B`
(isolated, and no bytecode written into KiCad's folders). On macOS:

    PY=/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3
    $PY -I -B xtask/freerouting-bench/score_board.py apply <board.kicad_pcb> <session.ses> [<more.ses> ...]
    $PY -I -B xtask/freerouting-bench/score_board.py measure <board.kicad_pcb> <out.json>

`apply` adds every wire and via of the session files to the board and saves
it **in place**, so give it a scratch copy, never a project you work in. It
does not refill zones: KiCad's standalone Python ignores `kicad_advanced`, so
its filler shows the 10.0.8 iterative-refill defect whatever the owner set.
Refill with `kicad-cli pcb drc --refill-zones --save-board` instead.

`measure` only reads. It writes one JSON file: every track segment and via
with its net, the pads per net, the copper zones, and a fingerprint of the
placement. The xtask does the arithmetic (categories, ratios, widths), so it
is unit-tested without KiCad.

One board per process: KiCad 10's Python bindings return a bare SWIG object
from the second `LoadBoard` in one process.
"""

import hashlib
import json
import re
import sys

import pcbnew


def tokens(text):
    # `(string_quote ")` names the quote character itself; drop it so the lone
    # quote does not swallow everything up to the next one.
    text = re.sub(r'\(string_quote\s+"\)', "", text)
    return re.findall(r'"[^"]*"|\(|\)|[^\s()]+', text)


def parse(tok, i=0):
    out = []
    while i < len(tok):
        t = tok[i]
        if t == "(":
            node, i = parse(tok, i + 1)
            out.append(node)
        elif t == ")":
            return out, i + 1
        else:
            out.append(t[1:-1] if t.startswith('"') else t)
            i += 1
    return out, i


def find(node, head):
    return [c for c in node if isinstance(c, list) and c and c[0] == head]


UNIT_NM = {"um": 1000.0, "mm": 1_000_000.0, "mil": 25_400.0, "inch": 25_400_000.0}


def session_routes(path):
    """The wires and vias of one session file, in nanometres with KiCad's Y
    (down), as (wires, vias): wires are (net, layer, width, [(x, y), ...]),
    vias are (net, x, y, diameter)."""
    tree, _ = parse(tokens(open(path, encoding="utf-8").read()))
    if not tree or not tree[0] or tree[0][0] != "session":
        raise SystemExit(f"{path}: not a Specctra session file")
    routes = find(tree[0], "routes")
    if not routes:
        raise SystemExit(f"{path}: no (routes ...) section")
    routes = routes[0]
    resolution = find(routes, "resolution")
    if not resolution or resolution[0][1] not in UNIT_NM:
        raise SystemExit(f"{path}: routes without a known (resolution unit n)")
    scale = UNIT_NM[resolution[0][1]] / float(resolution[0][2])

    def nm(value):
        return int(round(float(value) * scale))

    via_diameter = {}
    for library in find(routes, "library_out"):
        for padstack in find(library, "padstack"):
            circles = [c for shape in find(padstack, "shape") for c in find(shape, "circle")]
            if circles:
                via_diameter[padstack[1]] = max(nm(c[2]) for c in circles)
    wires, vias = [], []
    for network in find(routes, "network_out"):
        for net in find(network, "net"):
            name = net[1]
            for wire in find(net, "wire"):
                for path_node in find(wire, "path"):
                    layer, width = path_node[1], nm(path_node[2])
                    numbers = path_node[3:]
                    points = [
                        (nm(numbers[i]), -nm(numbers[i + 1]))
                        for i in range(0, len(numbers) - 1, 2)
                    ]
                    wires.append((name, layer, width, points))
            for via in find(net, "via"):
                if via[1] not in via_diameter:
                    raise SystemExit(f"{path}: via padstack {via[1]!r} has no circle in library_out")
                vias.append((name, nm(via[2]), -nm(via[3]), via_diameter[via[1]]))
    return wires, vias


def via_drill(board, net):
    """The via drill of the net's class, which a session file does not carry."""
    try:
        drill = net.GetNetClass().GetViaDrill()
        if drill > 0:
            return drill
    except Exception:  # noqa: BLE001 - a binding that lacks the call
        pass
    return board.GetDesignSettings().GetCurrentViaDrill()


def apply(board_path, session_paths):
    board = pcbnew.LoadBoard(board_path)
    nets = {}
    for code in range(board.GetNetCount()):
        net = board.FindNet(code)
        if net:
            nets[net.GetNetname()] = net
    tracks = vias = 0
    drills = set()
    for session in session_paths:
        wires, session_vias = session_routes(session)
        for name, layer, width, points in wires:
            if name not in nets:
                raise SystemExit(f"{session}: net {name!r} is not on the board")
            layer_id = board.GetLayerID(layer)
            if layer_id < 0 or not pcbnew.IsCopperLayer(layer_id):
                raise SystemExit(f"{session}: {layer!r} is not a copper layer of the board")
            for (x1, y1), (x2, y2) in zip(points, points[1:]):
                if (x1, y1) == (x2, y2):
                    continue
                track = pcbnew.PCB_TRACK(board)
                track.SetStart(pcbnew.VECTOR2I(x1, y1))
                track.SetEnd(pcbnew.VECTOR2I(x2, y2))
                track.SetWidth(width)
                track.SetLayer(layer_id)
                track.SetNet(nets[name])
                board.Add(track)
                tracks += 1
        for name, x, y, diameter in session_vias:
            if name not in nets:
                raise SystemExit(f"{session}: net {name!r} is not on the board")
            via = pcbnew.PCB_VIA(board)
            via.SetPosition(pcbnew.VECTOR2I(x, y))
            via.SetWidth(pcbnew.F_Cu, diameter)
            drill = via_drill(board, nets[name])
            via.SetDrill(drill)
            drills.add(drill)
            via.SetNet(nets[name])
            board.Add(via)
            vias += 1
    pcbnew.SaveBoard(board_path, board)
    print(json.dumps({"tracks": tracks, "vias": vias, "via_drills_nm": sorted(drills)}))


def measure(board_path, out_path):
    board = pcbnew.LoadBoard(board_path)
    pads = {}
    placement = []
    footprints = 0
    for footprint in board.GetFootprints():
        footprints += 1
        position = footprint.GetPosition()
        placement.append(
            (
                footprint.GetReference(),
                position.x,
                position.y,
                round(footprint.GetOrientationDegrees(), 3),
                footprint.GetLayer(),
            )
        )
        for pad in footprint.Pads():
            name = pad.GetNetname()
            if name and (pad.IsOnLayer(pcbnew.F_Cu) or pad.IsOnLayer(pcbnew.B_Cu) or pad.IsOnCopperLayer()):
                pads[name] = pads.get(name, 0) + 1
    tracks, vias = [], []
    for item in board.GetTracks():
        kind = item.GetClass()
        net = item.GetNetname()
        if kind == "PCB_VIA":
            position = item.GetPosition()
            vias.append([net, position.x, position.y, item.GetWidth(pcbnew.F_Cu), item.GetDrill()])
        else:
            start, end = item.GetStart(), item.GetEnd()
            tracks.append(
                [
                    net,
                    board.GetLayerName(item.GetLayer()),
                    item.GetWidth(),
                    start.x,
                    start.y,
                    end.x,
                    end.y,
                    int(round(item.GetLength())),
                ]
            )
    zones = []
    for zone in board.Zones():
        if zone.GetIsRuleArea():
            continue
        layers = [board.GetLayerName(layer) for layer in zone.GetLayerSet().Seq()]
        zones.append(
            {
                "net": zone.GetNetname(),
                "layers": layers,
                "name": zone.GetZoneName(),
                "priority": zone.GetAssignedPriority(),
                "outline_area_mm2": round(zone.Outline().Area() / 1e12, 3),
                "filled": bool(zone.IsFilled()),
            }
        )
    digest = hashlib.sha256(json.dumps(sorted(placement)).encode("utf-8")).hexdigest()
    result = {
        "kicad": pcbnew.Version(),
        "footprints": footprints,
        "placement": digest,
        "pads": pads,
        # [net, layer, width, x1, y1, x2, y2, length], all in nanometres.
        "tracks": tracks,
        # [net, x, y, diameter, drill], all in nanometres.
        "vias": vias,
        "zones": zones,
    }
    with open(out_path, "w", encoding="utf-8") as out:
        json.dump(result, out, ensure_ascii=False)
    print(json.dumps({"tracks": len(tracks), "vias": len(vias), "zones": len(zones)}))


def main(argv):
    if len(argv) >= 4 and argv[1] == "apply":
        apply(argv[2], argv[3:])
    elif len(argv) == 4 and argv[1] == "measure":
        measure(argv[2], argv[3])
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv)
