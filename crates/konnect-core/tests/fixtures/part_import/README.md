# Part import fixtures

Hand-written stand-ins for one part a user exported or downloaded in KiCad format, used by the
`import_library_part` tests (`src/tools/library_part_import_tests.rs`).

- `TEST_LDO.kicad_sym`: one synthetic 3-pin LDO symbol in the KiCad 9 format (`version 20241209`), whose
  Footprint field names a vendor library the way exported parts often do.
- `SOT-23_TEST.kicad_mod`: a matching 3-pad footprint. Its model reference points at a vendor path that
  does not exist and carries a non-default offset and rotation, so the tests can check that the transform
  is kept.
- `SOT-23_TEST.step`: a STEP header with an empty data section, enough to pass the tool's format sniff.

None of this data comes from EasyEDA, LCSC or any other vendor. EasyEDA's terms forbid automated
collection of its content, so no real response was recorded.
