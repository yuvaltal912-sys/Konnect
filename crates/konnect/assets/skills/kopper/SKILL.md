---
name: kopper
description: "One entry point to all of 1 Kopper's KiCad skills. Use it when the user calls 1 Kopper by name or wants every KiCad skill for a task: it loads the Konnect operating rules, then the workflow skills that fit the task (schematic, PCB layout and routing, review, library, manufacturing)."
argument-hint: "[KiCad task]"
---

# 1 Kopper: load the skills for this task

1 Kopper is Konnect's KiCad tools plus fastroute best-of-N routing. Its knowledge
is split across the skills below. This skill only chooses and loads them; it adds
no rules of its own. It is `kopper`, not `kopper1`, because `kopper1` is the
plugin's own name.

## Steps

1. **Load `konnect` first, always.** It holds the operating rules every KiCad task
   follows: work through the Konnect MCP tools, never edit `.kicad_*` files directly.
2. **Load each workflow skill the task touches**, from the table. When unsure,
   load it: a skill that turns out unneeded costs little, and a missed one costs a
   wrong board.
3. **Then do the task**, following the loaded skills. If the user gave no task,
   ask what they want done in KiCad.

Load a skill with your skill tool, by its name. Inside the `kopper1` plugin the
name may carry the plugin prefix, as in `kopper1:kicad-pcb`.

| The task involves | Load |
|---|---|
| Circuits, symbols placed on a sheet, wiring, nets, ERC | `kicad-schematic` |
| Board outline, placing footprints, routing, pours, vias, DRC fixes | `kicad-pcb` |
| Reviewing or auditing a design, "is it ready", DRC/ERC findings | `kicad-review` |
| Creating or fixing a symbol or footprint, library tables | `kicad-library` |
| Gerbers, drill files, BOM, pick-and-place, ordering from a fab | `kicad-manufacture` |
| SPICE simulation, only when it is installed (1 Kopper turns it off) | `kicad-simulation` |

A whole new board usually needs `kicad-schematic`, `kicad-pcb` and `kicad-review`,
in that order of work.
