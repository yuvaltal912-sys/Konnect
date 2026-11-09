"""Writes the layout-quality fixture boards with KiCad 10's own pcbnew.

Run (system python is the wrong interpreter):
  /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 -I make_fixtures.py

Each board holds a "good" and a "bad" case side by side so one file can carry
a positive and a negative control. README.md lists what is where.
"""
import os
import pcbnew
from pcbnew import FromMM as mm, VECTOR2I

HERE = os.path.dirname(os.path.abspath(__file__))
F, IN1, IN2, B = pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.In2_Cu, pcbnew.B_Cu


def v(x, y):
    return VECTOR2I(mm(x), mm(y))


class Fix:
    def __init__(self, layers=4, w=60, h=40):
        self.b = pcbnew.BOARD()
        self.b.SetCopperLayerCount(layers)
        self.nets = {}
        edge = pcbnew.PCB_SHAPE(self.b)
        edge.SetShape(pcbnew.SHAPE_T_RECT)
        edge.SetStart(v(0, 0))
        edge.SetEnd(v(w, h))
        edge.SetLayer(pcbnew.Edge_Cuts)
        edge.SetWidth(mm(0.05))
        self.b.Add(edge)

    def net(self, name):
        if name not in self.nets:
            n = pcbnew.NETINFO_ITEM(self.b, name)
            self.b.Add(n)
            self.nets[name] = n
        return self.nets[name]

    def part(self, ref, x, y, pads, size=(0.6, 0.6), rot=0):
        """pads: list of (number, dx, dy, net, pin_function|None)."""
        fp = pcbnew.FOOTPRINT(self.b)
        fp.SetReference(ref)
        fp.SetValue(ref)
        fp.SetPosition(v(x, y))
        for number, dx, dy, net, func in pads:
            p = pcbnew.PAD(fp)
            p.SetSize(v(*size))
            p.SetShape(pcbnew.PAD_SHAPE_RECT)
            p.SetAttribute(pcbnew.PAD_ATTRIB_SMD)
            p.SetLayerSet(p.SMDMask())
            p.SetNumber(str(number))
            p.SetPosition(v(x + dx, y + dy))
            if net:
                p.SetNet(self.net(net))
            if func:
                p.SetPinFunction(func)
            fp.Add(p)
        if rot:
            fp.SetOrientationDegrees(rot)
        self.b.Add(fp)
        return fp

    def track(self, net, layer, pts, width=0.2):
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            t = pcbnew.PCB_TRACK(self.b)
            t.SetStart(v(x0, y0))
            t.SetEnd(v(x1, y1))
            t.SetWidth(mm(width))
            t.SetLayer(layer)
            t.SetNet(self.net(net))
            self.b.Add(t)

    def via(self, net, x, y, size=0.6, drill=0.3):
        via = pcbnew.PCB_VIA(self.b)
        via.SetPosition(v(x, y))
        via.SetWidth(mm(size))
        via.SetDrill(mm(drill))
        via.SetViaType(pcbnew.VIATYPE_THROUGH)
        via.SetLayerPair(F, B)
        via.SetNet(self.net(net))
        self.b.Add(via)

    def zone(self, net, layer, rect, priority=0):
        x0, y0, x1, y1 = rect
        z = pcbnew.ZONE(self.b)
        z.SetLayer(layer)
        z.SetNet(self.net(net))
        z.SetAssignedPriority(priority)
        z.SetMinThickness(mm(0.2))
        z.SetLocalClearance(mm(0.3))
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
        ol = z.Outline()
        ol.NewOutline()
        for x, y in ((x0, y0), (x1, y0), (x1, y1), (x0, y1)):
            ol.Append(mm(x), mm(y))
        self.b.Add(z)
        return z

    def keepout(self, layer, rect):
        x0, y0, x1, y1 = rect
        z = pcbnew.ZONE(self.b)
        z.SetIsRuleArea(True)
        z.SetDoNotAllowZoneFills(True)
        z.SetDoNotAllowTracks(False)
        z.SetDoNotAllowVias(False)
        z.SetDoNotAllowPads(False)
        z.SetDoNotAllowFootprints(False)
        z.SetLayer(layer)
        ol = z.Outline()
        ol.NewOutline()
        for x, y in ((x0, y0), (x1, y0), (x1, y1), (x0, y1)):
            ol.Append(mm(x), mm(y))
        self.b.Add(z)

    def save(self, name):
        pcbnew.ZONE_FILLER(self.b).Fill(self.b.Zones())
        path = os.path.join(HERE, name)
        pcbnew.SaveBoard(path, self.b)
        print("wrote", path)


_n = [0]


def pin(f, ref, x, y, net, size=0.8):
    """A one-pad part centred on (x, y)."""
    f.part(ref, x, y, [(1, 0, 0, net, None)], size=(size, size))


def gnd(f, x, y):
    """A ground pad with a via in it, tying it to every ground plane."""
    _n[0] += 1
    pin(f, "G%d" % _n[0], x, y, "GND", 1.0)
    f.via("GND", x, y)


# ── 1. Reference-plane continuity ───────────────────────────────────────────
# In1.Cu: a GND plane with a 3V3 region on the right and a 3 mm slot.
# CLK_OK runs over solid GND; CLK_SPLIT crosses the slot and the 3V3 region.
def plane_split():
    f = Fix(4)
    f.zone("GND", IN1, (1, 1, 59, 39), priority=0)
    f.zone("3V3", IN1, (40, 21, 59, 39), priority=1)
    f.keepout(IN1, (20, 21, 23, 39))
    f.zone("GND", B, (1, 1, 59, 39))
    pin(f, "U1", 6, 10, "CLK_OK")
    pin(f, "U2", 54, 10, "CLK_OK")
    f.track("CLK_OK", F, [(6, 10), (54, 10)], 0.2)
    pin(f, "U3", 6, 30, "CLK_SPLIT")
    pin(f, "U4", 54, 30, "CLK_SPLIT")
    f.track("CLK_SPLIT", F, [(6, 30), (54, 30)], 0.2)
    gnd(f, 3, 3)
    gnd(f, 57, 3)
    gnd(f, 3, 37)
    f.save("plane_split.kicad_pcb")


# ── 2. Layer change between different reference planes ──────────────────────
# F.Cu references In1 (GND), B.Cu references In2 (GND). CLK_STITCHED changes
# layer with a GND via 1.5 mm away; CLK_BARE has none within 20 mm.
def layer_change():
    f = Fix(4)
    f.zone("GND", IN1, (1, 1, 59, 39))
    f.zone("GND", IN2, (1, 1, 59, 39))
    for ref, y, net, stitch in (("A", 10, "CLK_STITCHED", True), ("B", 30, "CLK_BARE", False)):
        pin(f, "U%s1" % ref, 6, y, net)
        pin(f, "U%s2" % ref, 54, y, net)
        f.track(net, F, [(6, y), (25, y)], 0.2)
        f.via(net, 25, y)
        f.track(net, B, [(25, y), (54, y)], 0.2)
        f.via(net, 54, y)
        if stitch:
            f.via("GND", 26.5, y + 1.5)
            f.via("GND", 52.5, y + 1.5)
    gnd(f, 3, 3)
    gnd(f, 57, 37)
    f.save("layer_change.kicad_pcb")


# ── 3. Decoupling loops ─────────────────────────────────────────────────────
# U1: capacitor 1.6 mm from the pin, ground through a via beside both.
# U2: capacitor 12 mm away on a long route; its ground goes round through a far via.
def decoupling():
    f = Fix(4)
    f.zone("GND", IN1, (1, 1, 59, 39))
    f.part("U1", 15, 10, [(1, -1.0, 0, "3V3", "VDD"), (2, 1.0, 0, "GND", "GND")], size=(0.8, 0.8))
    f.part("C1", 15, 12.5, [(1, -1.0, 0, "3V3", None), (2, 1.0, 0, "GND", None)])
    f.track("3V3", F, [(14, 10), (14, 12.5)], 0.25)
    f.track("GND", F, [(16, 10), (16, 12.5)], 0.25)
    f.via("GND", 16, 11.2)
    f.part("U2", 15, 28, [(1, -1.0, 0, "3V3_B", "VDD"), (2, 1.0, 0, "GND", "GND")], size=(0.8, 0.8))
    f.part("C2", 27, 28, [(1, -1.0, 0, "3V3_B", None), (2, 1.0, 0, "GND", None)])
    f.track("3V3_B", F, [(14, 28), (14, 33), (26, 33), (26, 28)], 0.25)
    f.track("GND", F, [(28, 28), (28, 36)], 0.25)
    f.via("GND", 28, 36)
    f.via("GND", 16, 28)
    f.save("decoupling.kicad_pcb")


# ── 4. Neck-down ────────────────────────────────────────────────────────────
# P5V_NECK: 2 mm track with a 3 mm stretch at 0.2 mm. P5V_WIDE: 2 mm throughout.
def neck_down():
    f = Fix(2)
    for tag, y, neck in (("NECK", 10, True), ("WIDE", 30, False)):
        net = "P5V_" + tag
        pin(f, "J" + tag[0], 6, y, net, 2.5)
        pin(f, "U" + tag[0], 54, y, net, 2.5)
        if neck:
            f.track(net, F, [(6, y), (25, y)], 2.0)
            f.track(net, F, [(25, y), (28, y)], 0.2)
            f.track(net, F, [(28, y), (54, y)], 2.0)
        else:
            f.track(net, F, [(6, y), (54, y)], 2.0)
    f.zone("GND", B, (1, 1, 59, 39))
    gnd(f, 3, 37)
    f.save("neck_down.kicad_pcb")


# ── 5. Buck hot loops ───────────────────────────────────────────────────────
# U1: tight input loop, feedback far from the switch node.
# U2: input capacitor far away, feedback track running beside the switch node.
def buck():
    f = Fix(4, w=90, h=40)
    f.zone("GND", IN1, (1, 1, 89, 39))
    for tag, ox, far in (("1", 5, False), ("2", 45, True)):
        vin, sw, vout, fb = "VIN" + tag, "SW" + tag, "VOUT" + tag, "FB" + tag
        x = ox + 8
        f.part("U" + tag, x, 20, [
            (1, -1.0, -1.0, vin, "VIN"), (2, 1.0, -1.0, sw, "SW"),
            (3, -1.0, 1.0, "GND", "PGND"), (4, 1.0, 1.0, fb, "FB")], size=(0.8, 0.8))
        cx, cy = (x - 4, 20) if not far else (x - 4, 33)
        f.part("C" + tag + "1", cx, cy, [(1, -1.0, 0, vin, None), (2, 1.0, 0, "GND", None)])
        f.part("L" + tag, x + 9, 19, [(1, -1.5, 0, sw, None), (2, 1.5, 0, vout, None)], size=(1.2, 1.2))
        f.part("C" + tag + "2", x + 14, 22, [(1, 0, -1.0, vout, None), (2, 0, 1.0, "GND", None)])
        f.track(sw, F, [(x + 1, 19), (x + 7.5, 19)], 0.8)
        f.track(vout, F, [(x + 10.5, 19), (x + 14, 19), (x + 14, 21)], 0.8)
        if not far:
            f.track(vin, F, [(x - 5, 20), (x - 5, 19), (x - 1, 19)], 0.6)
            f.track("GND", F, [(x - 3, 20), (x - 3, 21), (x - 1, 21)], 0.6)
            f.via("GND", x - 3, 21)
        else:
            f.track(vin, F, [(x - 5, 33), (x - 5, 28), (x - 1, 19)], 0.6)
            f.track("GND", F, [(x - 3, 33), (x - 3, 35)], 0.6)
            f.via("GND", x - 3, 35)
            f.track("GND", F, [(x - 1, 21), (x - 1, 23)], 0.6)
            f.via("GND", x - 1, 23)
        f.track("GND", F, [(x + 14, 23), (x + 14, 24)], 0.6)
        f.via("GND", x + 14, 24)
        if not far:
            f.track(fb, F, [(x + 1, 21), (x + 1, 27), (x + 14, 27)], 0.15)
        else:
            f.track(fb, F, [(x + 1, 21), (x + 2.1, 19.9), (x + 7, 19.9)], 0.15)
    f.save("buck.kicad_pcb")


# ── 6. Routing quality ──────────────────────────────────────────────────────
def routing():
    f = Fix(2, w=70, h=50)
    f.zone("GND", B, (1, 1, 69, 49))
    gnd(f, 3, 47)
    gnd(f, 67, 47)
    # CLEAN: one straight track.
    pin(f, "R1", 5, 5, "N_CLEAN")
    pin(f, "R2", 25, 5, "N_CLEAN")
    f.track("N_CLEAN", F, [(5, 5), (25, 5)])
    # CORNERS: right-angle and acute bends.
    pin(f, "R3", 5, 15, "N_CORNER")
    pin(f, "R4", 25, 15, "N_CORNER")
    f.track("N_CORNER", F, [(5, 15), (15, 15), (15, 18), (20, 18), (20, 12), (22, 15), (25, 15)])
    # DETOUR: pads 20 mm apart joined by a long loop with two vias.
    pin(f, "R5", 5, 25, "N_DETOUR")
    pin(f, "R6", 25, 25, "N_DETOUR")
    f.track("N_DETOUR", F, [(5, 25), (5, 33), (12, 33)])
    f.via("N_DETOUR", 12, 33)
    f.track("N_DETOUR", B, [(12, 33), (12, 40), (30, 40), (30, 33)])
    f.via("N_DETOUR", 30, 33)
    f.track("N_DETOUR", F, [(30, 33), (30, 25), (25, 25)])
    # STUB: a dangling spur.
    pin(f, "R7", 45, 5, "N_STUB")
    pin(f, "R8", 65, 5, "N_STUB")
    f.track("N_STUB", F, [(45, 5), (65, 5)])
    f.track("N_STUB", F, [(55, 5), (55, 9)])
    # PARALLEL: two unrelated signals 0.25 mm apart for 20 mm.
    pin(f, "R9", 45, 15, "N_PAR_A", 0.3)
    pin(f, "R10", 65, 15, "N_PAR_A", 0.3)
    f.track("N_PAR_A", F, [(45, 15), (65, 15)], 0.2)
    pin(f, "R11", 45, 15.45, "N_PAR_B", 0.2)
    pin(f, "R12", 65, 15.45, "N_PAR_B", 0.2)
    f.track("N_PAR_B", F, [(45, 15.45), (65, 15.45)], 0.2)
    # SPACED: the same two signals 2 mm apart.
    pin(f, "R13", 45, 25, "N_SP_A")
    pin(f, "R14", 65, 25, "N_SP_A")
    f.track("N_SP_A", F, [(45, 25), (65, 25)], 0.2)
    pin(f, "R15", 45, 27.2, "N_SP_B")
    pin(f, "R16", 65, 27.2, "N_SP_B")
    f.track("N_SP_B", F, [(45, 27.2), (65, 27.2)], 0.2)
    # DIFF pairs: good (constant 0.2 gap) and bad (the gap opens up).
    pin(f, "D1", 5, 44, "DG_P", 0.2)
    pin(f, "D2", 25, 44, "DG_P", 0.2)
    pin(f, "D3", 5, 44.4, "DG_N", 0.2)
    pin(f, "D4", 25, 44.4, "DG_N", 0.2)
    f.track("DG_P", F, [(5, 44), (25, 44)], 0.2)
    f.track("DG_N", F, [(5, 44.4), (25, 44.4)], 0.2)
    pin(f, "D5", 35, 40, "DB_P", 0.2)
    pin(f, "D6", 55, 40, "DB_P", 0.2)
    pin(f, "D7", 35, 44, "DB_N", 0.2)
    pin(f, "D8", 55, 44, "DB_N", 0.2)
    f.track("DB_P", F, [(35, 40), (55, 40)], 0.2)
    f.track("DB_N", F, [(35, 44), (40, 44), (44, 41), (48, 41), (51, 44), (55, 44)], 0.2)
    f.save("routing.kicad_pcb")


if __name__ == "__main__":
    plane_split()
    layer_change()
    decoupling()
    neck_down()
    buck()
    routing()
