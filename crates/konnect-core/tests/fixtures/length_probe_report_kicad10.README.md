# Length probe reports (Row LT, step 0)

`length_probe_report_kicad10.json` and `length_probe_report_ignored_kicad10.json` are
`kicad-cli pcb drc` reports, byte for byte as KiCad 10.0.8 wrote them on 2026-11-20. They
are the fixtures of `length_probe_tests.rs`: the probe's reader (`actual_mm`,
`parse_probe`) is held to what KiCad really prints, not to a message written by hand.

## What produced them

- **Board:** a judged scratch copy of the public fixture `specctra_pours_routing`, routed
  by fastroute x18 at effort Fast (nets `S1` to `S6`, plus `GND`).
- **Command:** `kicad-cli pcb drc --units mm --format json --output <report> <board>`,
  with `KICAD_CONFIG_HOME` on a scratch folder.
- **Rules:** a `.kicad_dru` with seven probe rules, `__konnect_len_0001` to
  `__konnect_len_0007`, one per net in the order `S1` to `S6`, `GND`. Each is
  `(constraint length (min 2000mm))`, `(severity error)`, condition
  `A.NetName == '<net>'`.
- **`length_probe_report_kicad10.json`:** the project lists no rule severities, so every
  check ran: 7 `length_out_of_range` entries and 13 others.
- **`length_probe_report_ignored_kicad10.json`:** the project set every other check to
  `ignore` and left `length_out_of_range` as `error`: only the 7 probe entries remain.
  Which item of a net KiCad names (a track, a via or a pad) varies from run to run.

## What they show

- One `length_out_of_range` entry per probed net, not in rule order, with the net as
  `[<net>]` in its items.
- The actual length is the number of the message that is not 2000: 52.2760, 37.5011,
  36.3050, 54.2499, 33.1560, 12.3816 mm for `S1` to `S6`.
- A net with pads and no track (`GND`) reads `actual 0.0000 mm`.
