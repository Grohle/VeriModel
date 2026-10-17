# VeriModel examples

These files show how to set up VeriModel for three kinds of project. They are
**illustrative**: they show the format and the most common settings. They are
not recommendations for any real project, and the rates are placeholders, not
market prices. Copy an example, then change every value to match your own
project and your own requirements.

## What is here

| File | What it is |
| --- | --- |
| `residential/project.yaml` | A residential building. Any IFC schema. Walls and doors must carry their fire rating; classification is Uniclass and optional; quantities are priced with `rates.csv`. |
| `office/project.yaml` | An office building in Spanish (`es`), imperial units, IFC4 only, OmniClass classification. Tighter tolerances. Slabs and columns must state `LoadBearing`. Quantities are grouped by classification. No prices. |
| `infrastructure/project.yaml` | A bridge and road project in IFC4X3. Very large tolerances, no storeys, quantities grouped by material. Uses a custom rule pack. |
| `infrastructure/infrastructure_rules.yaml` | A **custom** rule pack, in the same format as the built-in `crates/core/rule_packs/generic.yaml`. Referenced by the infrastructure `project.yaml`. |
| `sample.ids` | A small buildingSMART IDS 1.0 file with three specifications: walls have a fire rating, walls state `IsExternal`, beams have a material. Used with `--ids`. |
| `rates.csv` | Twelve unit rates in EUR. Used by the residential example through `qto.unit_rates_file`. |

Every `project.yaml` is commented section by section. The complete list of keys,
types and defaults is in [`docs/CONFIG.md`](../docs/CONFIG.md).

## How to run

Run these from the repository root. The IFC files are the public buildingSMART
samples in `tests/fixtures/` (see `tests/fixtures/README.md`).

Check that a configuration file is valid (unknown keys are errors):

```
verimodel validate-config --config examples/residential/project.yaml
```

Residential example, with the IDS file and the rates:

```
verimodel check --ifc tests/fixtures/bsi_ifc2x3_building_architecture.ifc \
  --config examples/residential/project.yaml --ids examples/sample.ids --out ./out
```

Quantity takeoff only, with prices from `rates.csv`:

```
verimodel qto --ifc tests/fixtures/bsi_ifc2x3_building_architecture.ifc \
  --config examples/residential/project.yaml --out ./out
```

Office example (IFC4 model, because the office project accepts IFC4 only):

```
verimodel check --ifc tests/fixtures/bsi_ifc4_building_structural.ifc \
  --config examples/office/project.yaml --out ./out
```

Infrastructure example (IFC4X3 bridge model, custom rule pack):

```
verimodel check --ifc tests/fixtures/bsi_ifc4x3_infra_bridge.ifc \
  --config examples/infrastructure/project.yaml --out ./out
```

If the `verimodel` binary is not installed, replace `verimodel` with
`cargo run -q -p verimodel-cli --`. To check that IFC reading and IDS checking
are installed, run `verimodel doctor`.

Exit codes: `0` = PASS or PASS WITH WARNINGS, `1` = FAIL, `2` = error (bad
input, missing file, missing IFC tooling), `3` = feature not implemented yet
(clash).

Each run writes a folder under `--out`:

- `report.html` (readable report), `report.json` (same data, machine readable)
- `failures.csv` (one row per element per failed rule)
- `qto.csv`, `qto.xlsx`, `qto.json` (quantities, and cost when rates match)
- `issues.bcfzip` (BCF issues for ERROR and WARNING findings) and `issues.csv`
- `run.json` (run ledger: input and config hashes, score, verdict) and `audit.log`
- `project.yaml` (the effective configuration of the run, so it can be reproduced)

## Unit rates: `rates.csv`

The file has one header line and then one line per rate. Its first line must be
exactly this header (column names are not case sensitive, and their order is
free):

```
code,description,unit,rate,currency,match_type,match_storey,match_classification
```

| Column | Required | Meaning |
| --- | --- | --- |
| `code` | yes | Your own identifier for the rate, shown in the cost table. |
| `description` | yes | What the rate covers, in plain words. Avoid commas, or wrap the text in double quotes. |
| `unit` | yes | Unit the rate is per. One of `m3` (cubic metres), `m2` (square metres), `m` (metres), `kg` (kilograms), `nr` (number of elements). |
| `rate` | yes | Price per unit. A plain number with a dot as decimal separator. No currency symbol. |
| `currency` | no | Currency of this rate, for example `EUR`. If empty, `qto.currency` from the project file is used. |
| `match_type` | no | IFC type the rate applies to, for example `IfcWall`. It also matches the subtypes of that type. |
| `match_storey` | no | Exact storey name, for example `Level 1`. Empty matches every storey. |
| `match_classification` | no | Start of the classification code, for example `Ss_25`. Empty matches every element. |

How a rate is chosen:

1. Quantities are grouped as set by `qto.group_by` (for example by IFC type and storey).
2. For each group, the **first rate in file order** whose match columns apply to
   every element of that group is used. An empty match column matches anything.
3. The quantity used is the group total in the rate's unit. For `nr`, it is the
   number of elements.
4. Amount = quantity × rate, rounded to two decimals.
5. A group with no matching rate has no cost. Its quantity is still reported.
6. If the matched rates use different currencies, no grand total is given.

Because the first match wins, put the more specific rates (for example one for
a storey) above the general rates (for example one for the whole type).

The twelve rates in `rates.csv` each match one IFC type (`IfcWall`, `IfcSlab`,
`IfcBeam`, `IfcColumn`, `IfcFooting`, `IfcDoor`, `IfcWindow`, `IfcRoof`,
`IfcCovering`, `IfcDuctSegment`, `IfcPipeSegment`, `IfcRailing`), so no two
rates compete for the same group.

## Notes

- The IDS file in `sample.ids` is a teaching example. A real project needs its
  own IDS file, written from its own requirements.
- The residential `project.yaml` names the IDS file in `requirements.ids_file`,
  but the current command line reads the IDS only from `--ids`. Pass `--ids` as
  shown above.
- Classification code patterns in these examples check the shape of a code
  (for example `Ss_25_10_30` or `23-13 11 11`). They do not check that the code
  exists in the Uniclass or OmniClass tables.
- Clash detection is planned for a later version. The examples keep it off.
