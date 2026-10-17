# Assumptions and interpretations

This is a starter list of the interpretations VeriModel makes when it reads
IFC files, checks them and counts quantities. Each entry says what the code does
and where. It records behaviour, not an opinion about the standard. When the
code changes, update the entry in the same change.

Standards referenced: IFC (IFC2X3, IFC4, IFC4X3), IDS 1.0, BCF 2.1 and 3.0,
and the buildingSMART property set names (for example `Pset_WallCommon`).

## Units

1. **Internal units are SI.** Lengths are in metres, areas in square metres,
   volumes in cubic metres and weights in kilograms, whatever the file declares.
   The reader converts from the units in `IfcUnitAssignment` and keeps the
   original units in `ModelUnits` so that a display can convert back.
   (`crates/core/src/model.rs`, module documentation and `ModelUnits`.)
2. **Display units are not wired yet.** `project.units` is stored but no
   report uses it; reports show SI values. (See [CONFIG.md](CONFIG.md).)

## Schema

3. **Schema names are normalised before comparison.** The comparison is
   case-insensitive and ignores anything after an underscore, so
   `IFC4X3_ADD2` is `IFC4X3`. A dot is read as `X`, so `IFC4.3` is `IFC4X3`.
   `IFC4X0`, `IFC4X1` and `IFC4X2` are all treated as `IFC4`.
   (`normalise_schema` in `crates/core/src/rules/checks.rs`, with its unit tests.)
4. **IFC4X1 and IFC4X2 are treated as IFC4.** A project that lists `IFC4` accepts
   a file written in IFC4X1 or IFC4X2. This is a simplification made for the
   gate, not a statement that the schemas are identical.
5. **Empty `project.ifc_versions` accepts any schema.** (Rule SCH-001.)

## Identity

6. **A GlobalId is valid when it is syntactically valid.** It must be 22
   characters, every character must come from the IFC base64 alphabet
   (`0-9`, `A-Z`, `a-z`, `_`, `$`), and the first character must be `0` to `3`.
   The check does not decode the identifier or confirm that it was generated
   from a UUID. (`is_valid_global_id`, rule ID-002.)
7. **Duplicate GlobalIds are compared across the whole model**, including
   spatial elements. Each element that shares an id is reported.
   (Rule ID-001.)
8. **Empty names are not compared** for duplicate names. By default, names must
   be unique within the same type and storey. (`unique_name`, rule ID-003.)

## Spatial structure

9. **The spatial structure is checked by type name.** A requirement such as
   "a site must exist" is satisfied when the spatial tree contains a node of
   that IFC type, compared without regard to case. (`spatial_hierarchy`.)
10. **Distance from origin is measured to the bounding-box centre**, not to the
    element's placement point. (`Geometry::distance_from_origin_m`.)
11. **A model-level rule counts its scope.** A rule about the whole model (for
    example a required spatial structure) counts as the number of elements in its
    scope, or as one when the scope is empty. (`checked` in `engine::evaluate`.)

## Geometry and quantities

12. **Base quantities come before geometry.** For each quantity, the first
    base quantity found in the file's quantity sets is used. When none is
    present, the value is computed from the tessellated geometry. The lookup
    order is in `crates/core/src/model.rs`:
    - Volume: `NetVolume`, `GrossVolume`, `Volume`; then geometry.
    - Area: `NetArea`, `GrossArea`, `NetSideArea`, `GrossSideArea`,
      `NetFloorArea`, `GrossFloorArea`, `OuterSurfaceArea`, `Area`; then geometry.
    - Length: `Length`, `NetLength`, `GrossLength`, `Height`, `Depth`; then the
      longest side of the bounding box.
    - Weight: `NetWeight`, `GrossWeight`, `Weight`. There is no geometry fallback
      for weight; without a base quantity it is not reported.
13. **Quantity sets are searched in sorted order.** When the same quantity name
    appears in two sets, the set that sorts first is used.
14. **Takeoff items are non-spatial products.** Spatial elements (sites,
    buildings, storeys, spaces) are left out, and so are feature elements
    (openings and similar), virtual elements, annotations and grids.
    (`is_takeoff_item`, `EXCLUDED_FAMILIES` in `crates/core/src/qto.rs`.)
15. **An element with no quantity at all still counts** in its group, adds
    nothing to the totals, and is reported as "no quantity".
16. **A degenerate solid is one with no triangles, or with volume and area both
    at or below the configured minimums.** (Rule GEO-002, `non_degenerate_geometry`.)
17. **Geometry rules are skipped without the geometry kernel.** A skipped rule
    gives a plain-language reason and is never counted as passed. (`RuleStatus::Skipped`.)

## Pricing (QTO)

18. **Group keys.** A `classification` key is the element's first non-empty
    classification code. A `material` key joins all of the element's materials with
    ` | `. A missing value becomes a placeholder such as `(unclassified)`.
19. **A rate applies to a group only when every element of the group matches
    every non-empty match column.** `match_type` also matches subtypes. `match_storey`
    is an exact storey name. `match_classification` is a prefix of the first
    classification code.
20. **The first matching rate in file order wins.** The rest of the table is not
    consulted for that group. (`rate_matches`, `compute` in `qto.rs`.)
21. **Quantity for pricing:** the group total in the rate's unit. For `nr` it is the
    number of elements. Amount = quantity × rate, rounded to two decimals.
22. **Totals are sums of the rounded amounts.** When matched rates use more than
    one currency, no grand total is given.
23. **The currency of a rate with no currency column** is `qto.currency`.

## Scoring and verdict

24. **Verdict.** `FAIL` if any finding is an error; `PASS_WITH_WARNINGS` if there
    are warnings but no errors; `PASS` otherwise. Informational findings do not
    change the verdict. (`crates/core/src/score.rs`, `VERDICT_RULE`.)
25. **Score.** `100 × (1 − (errors + 0.25 × warnings) / element checks)`,
    rounded to a whole number and clamped to 0 to 100. `element checks` is the
    sum of the elements each non-skipped rule evaluated. Informational findings do
    not change the score. With no element checks, the score is 0 if there is an
    error and 100 otherwise. (`score::summarise`.)
26. **Skipped rules never count as passed** and are not in the element-check
    denominator.
27. **A rule that applies to no element is "not applicable"**, not a pass.
    (`RuleStatus::NotApplicable`.)

## Rule evaluation

28. **A rule whose `enabled_if` path is missing is off.** The missing path is
    treated as `false`, and such a rule is left out of the report entirely.
29. **`checks.rules` overrides `checks.categories`**, which overrides the default
    (on). (`ChecksSection::is_enabled`.)
30. **Results are ordered by rule id**, and findings by severity, then rule id,
    then GlobalId, then message. (`engine::evaluate`, `RunReport::normalise`.)
31. **A property set requirement with no `applies_to`** applies to every
    non-spatial element. (`rules_for_required_pset`.)

## IDS

32. **Every IDS-derived finding is an error.** The cardinality of a specification
    (from `minOccurs` and `maxOccurs` on its applicability) decides only whether
    applicable elements must, may or must not exist. A requirement that fails on an
    applicable element is always an error. (`severity_for` in `crates/core/src/ids.rs`;
    cardinality from `run_ids` in `crates/ifc_reader/python/verimodel_extract.py`.)
33. **An IDS requirement that passes with zero applicable elements** is reported
    as not applicable. See [IDS.md](IDS.md).

## Outputs and reproducibility

34. **Deterministic outputs.** `report.json`, `report.html`, `failures.csv` and
    `qto.csv` are written so that the same inputs give the same bytes; the report
    writer does not write timestamps. Time stamps appear in `run.json` (timestamp and
    duration), `audit.log`, and the `CreationDate` of BCF topics, which is the real
    UTC time of the export. Other BCF content is repeatable. (`run.rs` and `bcf.rs`
    module documentation.)
35. **Hashes are recorded in `run.json`:** the IFC file (SHA-256 of the whole
    file), the IDS file, the rates file, the effective configuration (the
    `project.yaml` written to `out/`), and `report.json`.
36. **The effective configuration is written to `out/project.yaml`.** It is the
    configuration after defaults are applied, so a run can be reproduced from
    that folder.
37. **Every run appends to `audit.log`.** The log is never rewritten. Each line
    holds a UTC timestamp, the event and its details.

## Not implemented yet

- **Clash detection.** `clash.enabled: true` returns an explicit "not
  implemented" message. No clash is reported.
- **Display unit conversion** from `project.units`.
- **`output.include_3d_snapshot`** and the `clash.*` filters are stored but not read.
- **`requirements.ids_file`** is stored but not read; the command line uses `--ids`.
- **Classification rule CL-002 with `allowed_codes_file`** reads the file only when
  checks run, so a bad path is reported at run time, not when the file is validated.
