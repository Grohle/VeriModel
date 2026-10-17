# IDS in VeriModel

## What IDS is

IDS (Information Delivery Specification) is the buildingSMART standard for
writing down, in a machine-readable file, which information a model must
contain. An IDS file is XML. Each **specification** has two parts:

- **Applicability**: which elements the specification applies to, for example
  every `IFCWALL`.
- **Requirements**: what those elements must contain. The facets are `entity`,
  `attribute`, `property`, `classification`, `material` and `partOf`. Each
  facet has a cardinality: `required`, `optional` or `prohibited`.

A project can therefore state a requirement once, in IDS, and any tool that
reads IDS can check a model against it.

Standard and tools:

- IDS repository and documentation: <https://github.com/buildingSMART/IDS>
- IDS 1.0 schema (the XSD that defines the file format): <http://standards.buildingsmart.org/IDS/1.0/ids.xsd>
- IfcTester, the Python library in IfcOpenShell that reads and checks IDS: <http://ifcopenshell.org/>

VeriModel does not check IDS itself. It uses IfcTester (through the reader) to
audit the model, and turns the results into rules and findings.

## A minimal IDS file, walked through

The file used by the examples is [`examples/sample.ids`](../examples/sample.ids).
Its structure, from the outside in:

```xml
<ids:ids xmlns="http://standards.buildingsmart.org/IDS"
         xmlns:ids="http://standards.buildingsmart.org/IDS"
         xmlns:xs="http://www.w3.org/2001/XMLSchema"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://standards.buildingsmart.org/IDS http://standards.buildingsmart.org/IDS/1.0/ids.xsd">
  <ids:info>                       <!-- who, what, when; title is required -->
    <ids:title>VeriModel sample requirements</ids:title>
    <ids:version>1.0</ids:version>
    <ids:author>examples@example.com</ids:author>   <!-- must look like an e-mail address -->
    <ids:date>2026-10-17</ids:date>                 <!-- xs:date -->
    <ids:purpose>...</ids:purpose>
  </ids:info>
  <ids:specifications>
    <ids:specification name="Walls have a fire rating"
                       ifcVersion="IFC2X3 IFC4 IFC4X3_ADD2">   <!-- name and ifcVersion are required -->
      <ids:applicability minOccurs="0" maxOccurs="unbounded">  <!-- which elements -->
        <ids:entity>
          <ids:name><ids:simpleValue>IFCWALL</ids:simpleValue></ids:name>
        </ids:entity>
      </ids:applicability>
      <ids:requirements>                                        <!-- what they must have -->
        <ids:property dataType="IFCLABEL" cardinality="required">
          <ids:propertySet><ids:simpleValue>Pset_WallCommon</ids:simpleValue></ids:propertySet>
          <ids:baseName><ids:simpleValue>FireRating</ids:simpleValue></ids:baseName>
        </ids:property>
      </ids:requirements>
    </ids:specification>
    <!-- two more specifications in the sample: IsExternal (IFCBOOLEAN), and
         beams must have a material (<ids:material cardinality="required"/>) -->
  </ids:specifications>
</ids:ids>
```

Points to know when writing one:

- **Entity names are upper case** (`IFCWALL`, `IFCBEAM`), as the standard expects.
- **`ifcVersion` takes the tokens `IFC2X3`, `IFC4` and `IFC4X3_ADD2`**. The IDS
  1.0 schema does not accept `IFC4X3`; the IFC 4.3 token is `IFC4X3_ADD2`. A file
  with the wrong token is rejected by the schema check (verified with the
  schema in IfcTester 0.9.0).
- **`dataType` is an upper-case IFC type name** such as `IFCLABEL`, `IFCBOOLEAN`
  or `IFCREAL`.
- **`minOccurs` on `applicability`** sets whether the specification needs
  applicable elements. `minOccurs="1"` (or more) is `required`: at least one
  applicable element must exist, so a model with no walls fails a walls
  specification (VeriModel reports one model-level error). `minOccurs="0"` is
  `optional`: a model with no walls passes, and any wall that fails the
  requirement is still an error. The sample uses `minOccurs="0"` so that it runs
  across every fixture, including models with no walls or no beams. Use `"1"` when
  missing elements are themselves a failure.
- **Facet order** inside `requirements` may be mixed (the schema allows any
  order), but each facet element must have the children the schema names:
  `propertySet` and `baseName` for `property`; `name` for `attribute`; `system`
  for `classification`.

Check any IDS file against the schema and parse it with the same library that
VeriModel uses:

```python
import ifctester.ids as ids
spec = ids.open("examples/sample.ids", validate=True)   # XSD check, then parse
```

Run it against a model:

```python
import ifcopenshell, ifctester.reporter as reporter
model = ifcopenshell.open("tests/fixtures/bsi_ifc2x3_building_architecture.ifc")
spec.validate(model)                  # keep `model` alive until the report is made
r = reporter.Json(spec); r.report(); print(r.to_string())
```

### What the sample reports on the buildingSMART fixtures

Results from IfcTester 0.9.0 on the four files in `tests/fixtures/`:

| Fixture | Walls: FireRating | Walls: IsExternal | Beams: material | Specifications passing |
| --- | --- | --- | --- | --- |
| `bsi_ifc2x3_building_architecture.ifc` (4 walls, no beams) | fail (4 walls lack the property set) | fail (4 walls) | pass, vacuously (0 beams) | 1 of 3 |
| `bsi_ifc4_building_structural.ifc` (4 walls, 6 beams) | fail (4 walls) | fail (4 walls) | pass (6 beams) | 1 of 3 |
| `bsi_ifc4_building_hvac.ifc` (no walls, no beams) | pass, vacuously | pass, vacuously | pass, vacuously | 3 of 3 |
| `bsi_ifc4x3_infra_bridge.ifc` (4 walls, 8 beams) | fail (4 walls) | fail (4 walls) | pass (8 beams) | 1 of 3 |

"Vacuously" means the specification applies to no element, so nothing can fail.
VeriModel reports such a requirement as **not applicable**, not as a pass with
elements checked.

## How VeriModel reports IDS results

Source: `crates/core/src/ids.rs`. The IDS engine (IfcTester, driven by the reader)
returns the facts; the core turns them into rules and findings, so IDS results
appear in the same tables as rule-pack results.

- **One rule per requirement.** The rule id is `IDS-<spec>-<req>`, with both
  numbers counted from 1 and written with two digits: `IDS-01-01` is the first
  requirement of the first specification. Its category is `ids`.
- **A specification without requirements** gets one rule, `IDS-<spec>-00`, so
  its verdict is still reported.
- **Requirement quoted.** The rule's description is `<specification name>: <requirement text>`. Each finding
  puts the requirement text in `expected`, and the IDS engine's reason in
  `actual`. The requirement is therefore quoted verbatim in the report.
- **Severity is always error.** Every IDS-derived rule is an error, whatever the
  cardinality. A requirement that fails on an applicable element is a failure. The
  cardinality only says whether applicable elements must, may or must not exist
  (see the `minOccurs` note above).
- **Findings.** One finding per failed element, with its GlobalId, type and
  name. If a requirement failed but no element is named (for example a required
  specification with no applicable element), one model-level finding says so.
- **Status.** Pass, Fail, or Not applicable when the requirement passed with
  zero applicable elements.
- **Fix hint.** The specification's `instructions`, or
  `See IDS specification '<name>'.` when none is given.
- **Report header.** The IDS title, file name, and counts of specifications
  passed and total are written to the report and `run.json` (the IDS file's
  SHA-256 is recorded too).

Example: the IFC2X3 architecture fixture against the sample gives the walls
requirement `IDS-01-01` and four errors, one per wall. Each finding quotes the
requirement in `expected` (here `FireRating data shall be provided in the dataset
Pset_WallCommon`, the wording IfcTester generates) and gives the reason in
`actual` (here `The required property set does not exist`). The beams
specification has no applicable element in that file and reports `Not applicable`.

## Running with an IDS file

```
verimodel check --ifc tests/fixtures/bsi_ifc2x3_building_architecture.ifc \
  --config examples/residential/project.yaml --ids examples/sample.ids --out ./out
```

The command line reads the IDS from `--ids`. The `requirements.ids_file` key in
`project.yaml` is accepted but not read yet (see [CONFIG.md](CONFIG.md)).

## Where to go next

- The IDS 1.0 schema and repository (links above).
- The sample files used here are in `examples/sample.ids`, with notes in
  `examples/README.md`.
