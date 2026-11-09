"""Edit a Specctra DSN for routing experiments, before any of it is product code.

**Experiment only.** These are text edits of a DSN that Konnect exported, so a
routing idea can be measured with `cargo xtask freerouting-bench` before it is
built into the exporter. A DSN edited here has no reverse manifest: its result
is scored with `--score` (which reads the board), never imported with
`apply_specctra_ses`.

    python3 -I xtask/freerouting-bench/stage_merge.py classes  <in.dsn>
    python3 -I xtask/freerouting-bench/stage_merge.py planes   <in.dsn>
    python3 -I xtask/freerouting-bench/stage_merge.py unwiden  <in.dsn> <out.dsn> <net> [<net> ...]
    python3 -I xtask/freerouting-bench/stage_merge.py drop-plane <in.dsn> <out.dsn> <index> [<index> ...]
    python3 -I xtask/freerouting-bench/stage_merge.py layer-cost <in.dsn> <out.dsn> <layer> <preferred> <against>
    python3 -I xtask/freerouting-bench/stage_merge.py merge    <in.dsn> <stage1.ses> <out.dsn>

- `classes` lists each net class with its width and net count; `planes` lists
  each plane with its index, net, layer and area. Both only read.
- `unwiden` moves nets into the class with the narrowest width: a net that
  owns a pour then routes as a thin stub to it, not as a wide track.
- `drop-plane` removes planes by the index `planes` prints, so only the pours
  worth keeping reach the router.
- `layer-cost` sets one layer's trace costs (Freerouting's `layer_rule` in
  `autoroute_settings`), to steer signals off a layer that should stay ground.
- `merge` is staged routing: the wires and vias of a first run's session go
  into the DSN as fixed wiring, so a second run routes the rest around them.
  Freerouting keeps fixed wiring and does not write it to its session, so the
  stage-1 session is also copied to `<out>.fixed.ses`; the bench puts both
  sessions into the board when it scores the second run.

Inputs are never written. Plain Python 3, no KiCad.
"""

import re
import shutil
import sys

TOKEN = re.compile(r'\(string_quote\s+"\)|"[^"]*"|\(|\)|[^\s()]+')


def parse(text):
    """The text as nested lists of raw tokens: quotes are kept as written."""
    root, stack = None, []
    for token in TOKEN.findall(text):
        if token.startswith("(string_quote"):
            stack[-1].append(["string_quote", '"'])
        elif token == "(":
            stack.append([])
        elif token == ")":
            done = stack.pop()
            if stack:
                stack[-1].append(done)
            else:
                root = done
        else:
            stack[-1].append(token)
    if root is None or stack:
        raise SystemExit("not one balanced S-expression")
    return root


def render(node, depth=0):
    """Write the tree back, one list per line down to the fourth level."""
    if node[:1] == ["string_quote"]:
        return '(string_quote ")'
    if depth >= 4 or all(not isinstance(child, list) for child in node):
        return "(" + " ".join(render(c, depth + 1) if isinstance(c, list) else c for c in node) + ")"
    pad = "  " * (depth + 1)
    out, line = [], []
    for child in node:
        if isinstance(child, list):
            if line:
                out.append(" ".join(line))
                line = []
            out.append(render(child, depth + 1))
        else:
            line.append(child)
    if line:
        out.append(" ".join(line))
    return "(" + out[0] + "".join("\n" + pad + part for part in out[1:]) + "\n" + "  " * depth + ")"


def name(token):
    return token[1:-1] if token.startswith('"') else token


def children(node, head):
    return [c for c in node if isinstance(c, list) and c and c[0] == head]


def one(node, head):
    found = children(node, head)
    if len(found) != 1:
        raise SystemExit(f"expected one ({head} ...), found {len(found)}")
    return found[0]


def class_width(net_class):
    widths = [float(w[1]) for rule in children(net_class, "rule") for w in children(rule, "width")]
    return widths[0] if widths else None


def class_nets(net_class):
    """Indexes of the net names in a class: the atoms after its name."""
    return [i for i in range(2, len(net_class)) if not isinstance(net_class[i], list)]


def polygon_area(numbers):
    points = list(zip(numbers[0::2], numbers[1::2]))
    return abs(sum(a[0] * b[1] - b[0] * a[1] for a, b in zip(points, points[1:] + points[:1]))) / 2


def planes_of(structure):
    rows = []
    for index, plane in enumerate(children(structure, "plane")):
        polygon = one(plane, "polygon")
        numbers = [float(v) for v in polygon[3:]]
        rows.append((index, name(plane[1]), polygon[1], polygon_area(numbers), plane))
    return rows


def read(path):
    with open(path, encoding="utf-8") as source:
        return parse(source.read())


def write(path, tree):
    text = render(tree) + "\n"
    parse(text)  # what is written must read back
    with open(path, "x", encoding="utf-8") as out:
        out.write(text)


def cmd_classes(tree):
    for net_class in children(one(tree, "network"), "class"):
        print(f"{name(net_class[1])}: width {class_width(net_class)}, {len(class_nets(net_class))} nets")


def cmd_planes(tree):
    for index, net, layer, area, _ in planes_of(one(tree, "structure")):
        print(f"{index}: net {net}, layer {layer}, area {area:.0f}")


def cmd_unwiden(tree, nets):
    network = one(tree, "network")
    classes = [c for c in children(network, "class") if class_width(c) is not None]
    if not classes:
        raise SystemExit("the DSN has no net class with a width")
    base = min(classes, key=class_width)
    for net in nets:
        holders = [c for c in children(network, "class") if any(name(c[i]) == net for i in class_nets(c))]
        if len(holders) != 1:
            raise SystemExit(f"net {net!r} is in {len(holders)} classes, expected 1")
        holder = holders[0]
        if holder is base:
            raise SystemExit(f"net {net!r} is already in the narrowest class")
        index = next(i for i in class_nets(holder) if name(holder[i]) == net)
        token = holder.pop(index)
        base.insert(2, token)
        if not class_nets(holder):
            network.remove(holder)
        print(f"{net}: {name(holder[1])} (width {class_width(holder)}) -> {name(base[1])} (width {class_width(base)})")


def cmd_drop_plane(tree, indexes):
    structure = one(tree, "structure")
    rows = planes_of(structure)
    wanted = sorted({int(i) for i in indexes})
    if not wanted or wanted[0] < 0 or wanted[-1] >= len(rows):
        raise SystemExit(f"plane indexes are 0 to {len(rows) - 1}")
    for index in wanted:
        _, net, layer, area, plane = rows[index]
        structure.remove(plane)
        print(f"dropped plane {index}: net {net}, layer {layer}, area {area:.0f}")


def cmd_layer_cost(tree, layer, preferred, against):
    structure = one(tree, "structure")
    if not any(name(l[1]) == layer for l in children(structure, "layer")):
        raise SystemExit(f"the DSN has no layer {layer!r}")
    for value in (preferred, against):
        if not 0.1 <= float(value) <= 100:
            raise SystemExit("a trace cost is a number from 0.1 to 100")
    settings = children(structure, "autoroute_settings")
    if not settings:
        settings = [["autoroute_settings"]]
        structure.append(settings[0])
    if any(name(rule[1]) == layer for rule in children(settings[0], "layer_rule")):
        raise SystemExit(f"layer {layer!r} already has a layer_rule")
    settings[0].append(
        [
            "layer_rule",
            layer,
            ["active", "on"],
            ["preferred_direction_trace_costs", preferred],
            ["against_preferred_direction_trace_costs", against],
        ]
    )
    print(f"{layer}: trace costs {preferred} along and {against} against its preferred direction")


UNIT_UM = {"um": 1.0, "mm": 1000.0, "mil": 25.4, "inch": 25400.0}


def number(value):
    text = f"{value:.4f}".rstrip("0").rstrip(".")
    return "0" if text in ("", "-0") else text


def cmd_merge(tree, session):
    if session[0] != "session":
        raise SystemExit("the second file is not a Specctra session")
    routes = one(session, "routes")
    resolution = one(routes, "resolution")
    session_um = UNIT_UM[resolution[1]] / float(resolution[2])
    units = children(tree, "unit") or children(tree, "resolution")
    if not units or units[0][1] not in UNIT_UM:
        raise SystemExit("the DSN names no known unit")
    scale = session_um / UNIT_UM[units[0][1]]
    padstacks = {name(p[1]) for library in children(tree, "library") for p in children(library, "padstack")}
    layers = {name(l[1]) for l in children(one(tree, "structure"), "layer")}
    known = {name(net[1]) for net in children(one(tree, "network"), "net")}
    wiring = children(tree, "wiring")
    if not wiring:
        wiring = [["wiring"]]
        tree.append(wiring[0])
    wires = vias = 0
    nets = set()
    for network in children(routes, "network_out"):
        for net in children(network, "net"):
            if name(net[1]) not in known:
                raise SystemExit(f"the session routes net {name(net[1])!r}, which the DSN does not have")
            for wire in children(net, "wire"):
                for path in children(wire, "path"):
                    if name(path[1]) not in layers:
                        raise SystemExit(f"the session uses layer {path[1]!r}, which the DSN does not have")
                    values = [number(float(v) * scale) for v in path[2:]]
                    wiring[0].append(["wire", ["path", path[1]] + values, ["net", net[1]], ["type", "fix"]])
                    wires += 1
                    nets.add(name(net[1]))
            for via in children(net, "via"):
                if name(via[1]) not in padstacks:
                    raise SystemExit(f"the session uses via {via[1]!r}, which the DSN's library does not have")
                x, y = (number(float(v) * scale) for v in via[2:4])
                wiring[0].append(["via", via[1], x, y, ["net", net[1]], ["type", "fix"]])
                vias += 1
                nets.add(name(net[1]))
    if not wires and not vias:
        raise SystemExit("the session holds no wiring to merge")
    print(f"merged {wires} wires and {vias} vias of {len(nets)} nets as fixed wiring")


def main(argv):
    if len(argv) == 3 and argv[1] in ("classes", "planes"):
        (cmd_classes if argv[1] == "classes" else cmd_planes)(read(argv[2]))
        return
    if len(argv) >= 5 and argv[1] == "unwiden":
        tree = read(argv[2])
        cmd_unwiden(tree, argv[4:])
    elif len(argv) >= 5 and argv[1] == "drop-plane":
        tree = read(argv[2])
        cmd_drop_plane(tree, argv[4:])
    elif len(argv) == 7 and argv[1] == "layer-cost":
        tree = read(argv[2])
        cmd_layer_cost(tree, argv[4], argv[5], argv[6])
    elif len(argv) == 5 and argv[1] == "merge":
        tree = read(argv[2])
        cmd_merge(tree, read(argv[3]))
        write(argv[4], tree)
        shutil.copyfile(argv[3], re.sub(r"\.dsn$", "", argv[4]) + ".fixed.ses")
        return
    else:
        raise SystemExit(__doc__)
    write(argv[3], tree)


if __name__ == "__main__":
    main(sys.argv)
