"""Export unrouted Specctra DSN files from KiCad boards, for `cargo xtask freerouting-bench`.

Run it with KiCad's own Python, which has the `pcbnew` module (macOS):

    /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3 -I \
        xtask/freerouting-bench/export_kicad_dsn.py <folder of boards> <output folder>

Every `.kicad_pcb` below the input folder is copied into a scratch folder,
loaded there, stripped of its tracks, arcs and vias (footprints, zones and the
outline stay), and exported with KiCad's own Specctra exporter as
`<output>/<board name>.dsn`. The source boards are never opened for writing,
and nothing is saved back. This uses KiCad's exporter, not Konnect's, so the
corpus does not depend on any Konnect change.

Each board is exported in its own process: KiCad 10's Python bindings return a
bare SWIG object from the second `LoadBoard` in one process.
"""

import os
import shutil
import subprocess
import sys
import tempfile


def strip_routing(board):
    removed = 0
    for track in list(board.GetTracks()):
        board.Remove(track)
        removed += 1
    return removed


def export_one(copy, target):
    """Load one scratch copy, strip its routing, export it. Prints one line."""
    import pcbnew

    name = os.path.splitext(os.path.basename(copy))[0]
    board = pcbnew.LoadBoard(copy)
    footprints = len(board.GetFootprints())
    if footprints == 0:
        print(f"refuse {name}: no footprints")
        return 1
    removed = strip_routing(board)
    ok = pcbnew.ExportSpecctraDSN(board, target)
    if ok and os.path.exists(target):
        print(f"export {name}: {footprints} footprints, {removed} routing items stripped")
        return 0
    print(f"refuse {name}: KiCad's exporter returned {ok}")
    if os.path.exists(target):
        os.remove(target)
    return 1


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--one":
        sys.exit(export_one(sys.argv[2], sys.argv[3]))
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    source, output = (os.path.abspath(p) for p in sys.argv[1:])
    os.makedirs(output, exist_ok=True)
    boards = []
    for root, _dirs, files in os.walk(source):
        boards.extend(os.path.join(root, f) for f in files if f.endswith(".kicad_pcb"))
    boards.sort()
    exported, refused = 0, 0
    with tempfile.TemporaryDirectory() as scratch:
        for path in boards:
            name = os.path.splitext(os.path.basename(path))[0]
            target = os.path.join(output, name + ".dsn")
            if os.path.exists(target):
                print(f"skip   {name}: {target} exists")
                continue
            copy = os.path.join(scratch, name + ".kicad_pcb")
            shutil.copyfile(path, copy)
            child = subprocess.run(
                [sys.executable, "-I", os.path.abspath(__file__), "--one", copy, target],
                cwd=scratch,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=600,
            )
            line = child.stdout.strip().splitlines()[-1:] or [
                f"refuse {name}: exit {child.returncode}: "
                + " ".join(
                    l for l in child.stderr.splitlines() if "memory leak" not in l and "wxApp" not in l
                )[-300:]
            ]
            print(line[0])
            if child.returncode == 0 and os.path.exists(target):
                exported += 1
            else:
                refused += 1
    print(f"{exported} exported, {refused} refused, of {len(boards)} boards")


if __name__ == "__main__":
    main()
