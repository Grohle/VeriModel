# project.yaml reference

`project.yaml` holds everything that is specific to one project: its schema
rules, the properties every element must carry, tolerances, quantity settings,
naming and classification rules, rule switches and output settings. The
engine itself has no project-specific values; it reads them from this file.

This page is derived from `crates/core/src/config.rs`, the single source of
truth for the schema. If this page and the code disagree, the code wins.

## Rules that apply to the whole file

- **Every key is checked.** Unknown keys are errors, with the key name in the
  message. A typo cannot silently switch a rule off. Wrong types are errors too
  (for example `ID-003: no` where a boolean is expected).
- **Every key is optional.** A missing key takes its default. An empty file
  (`{}`) is a valid configuration with all defaults. A missing `--config` on the
  command line also means all defaults.
- **Relative paths** (`requirements.ids_file`, `requirements.rule_pack` when it
  is a file, `qto.unit_rates_file`, `classification.allowed_codes_file`) are
  resolved against the folder that contains `project.yaml`, not the folder you
  run the command from.
- **Validation** runs when the file is read. Problems are listed in plain
  language, separated by semicolons. Checks made by `Config::validate`:
  `qto.group_by` values and non-empty list; `tolerances.far_from_origin_m > 0`;
  `tolerances.geometry.min_volume_m3 >= 0`; `output.bcf_version` is `2.1` or
  `3.0`; every regular expression in `naming.patterns` and
  `classification.code_pattern` compiles; every `required_psets` entry has a
  non-empty `pset` and a valid `severity`; `checks.unique_name_within` values
  are `type` or `storey`.
- **Validated when the checks run:** every rule id in `checks.rules` must exist
  in the active rule pack (or be an `IDS-*` id) and every name in
  `checks.categories` must be a category of the pack or one of the standard
  categories (identity, spatial, properties, geometry, classification, naming,
  sanity, schema, ids); otherwise the run stops with a message naming the key.
  `allowed_codes_file` is read only when the checks run.
- **Check the file:** `verimodel validate-config --config project.yaml`.
  `verimodel validate-config --schema` prints the JSON schema, which editors
  can use for autocompletion.

Status markers used below: **read** means the current code uses the value;
**declared** means the key is accepted and stored in the file but no code uses
it yet.

## Top level

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `project` | section | see below | Project identity and file-level rules. |
| `requirements` | section | see below | Rule pack and required property sets. |
| `tolerances` | section | see below | Numbers that decide geometry and position checks. |
| `qto` | section | see below | Quantity takeoff grouping, rates and currency. |
| `clash` | section | see below | Clash detection (planned). |
| `output` | section | see below | Report language and BCF settings. |
| `checks` | section | see below | Switch single rules or whole categories off. |
| `naming` | section | see below | Naming convention per element type. |
| `classification` | section | see below | Classification codes elements must carry. |

## `project`

| Key | Type | Default | Status | Meaning |
| --- | --- | --- | --- | --- |
| `project.name` | text | `Generic Project` | read | Name shown in reports. |
| `project.language` | `en` or `es` | `en` | read | Language of the user interface and reports. |
| `project.units` | `auto`, `metric` or `imperial` | `auto` | declared | Intended unit preference for display. `auto` follows the IFC file. No code reads this key yet; all values are stored in SI units. |
| `project.classification_system` | text | `none` | read | Name of the classification system expected, for example `Uniclass`, `OmniClass`, `CCI`, `DIN276`. Rule CL-001 matches it, ignoring case, against the system name in the file. `none` disables that match. |
| `project.ifc_versions` | list of text | `[IFC2X3, IFC4, IFC4X3]` | read | Schemas the project accepts. Read by rule SCH-001. An empty list accepts any schema. Compared after normalisation (see [ASSUMPTIONS.md](ASSUMPTIONS.md)). |

## `requirements`

| Key | Type | Default | Status | Meaning |
| --- | --- | --- | --- | --- |
| `requirements.ids_file` | text (path) | none | used | IDS file to audit against, relative to the config file. `--ids` on the command line (or the IDS picked in the app) takes precedence. |
| `requirements.rule_pack` | text | `generic` | read | `generic` for the built-in pack, or a path to a custom rule pack YAML (relative to this file). See [RULES.md](RULES.md). |
| `requirements.required_psets` | list of `RequiredPset` | `[]` | read | Property sets, and optionally properties, that must exist. Each entry creates generated rules (see below). |

### `requirements.required_psets[]` (`RequiredPset`)

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `pset` | text | none, required | Property set name, for example `Pset_WallCommon`. Must not be empty. |
| `applies_to` | list of text | `[]` | IFC types (or supertypes) the rule applies to, for example `[IfcWall]`. Empty applies it to all non-spatial elements. |
| `properties` | list of text | `[]` | Property names that must exist inside the set and carry a value. Empty requires only that the set exists. |
| `severity` | `error`, `warning` or `info` | `error` | Severity of every finding the entry produces. |

Each entry generates rules with category `properties`. `n` is the position of
the entry, written with three digits (001, 002, ...), and `m` is the position of
the property, written with two digits (01, 02, ...):

- `CFG-PSET-n`: the property set must exist.
- `CFG-PSET-n-m`: the property must exist and have a value.

These ids can be switched off like any other rule id (`checks.rules`).

Example:

```yaml
requirements:
  required_psets:
    - pset: Pset_WallCommon
      applies_to: [IfcWall]
      properties: [FireRating, IsExternal]
      severity: error
```

## `tolerances`

| Key | Type | Default | Status | Meaning |
| --- | --- | --- | --- | --- |
| `tolerances.clash_hard_mm` | number (mm) | `10.0` | declared | Hard-clash distance for clash detection. Clash detection is not implemented yet. |
| `tolerances.clash_clearance_mm` | number (mm) | `50.0` | declared | Clearance distance for clash detection. Not implemented yet. |
| `tolerances.far_from_origin_m` | number (m), must be > 0 | `10000.0` | read | An element whose bounding-box centre is farther than this from the origin is flagged by rule SP-004 (generic pack). |
| `tolerances.geometry.min_volume_m3` | number (m³), must be >= 0 | `0.000001` | read | A solid with volume at or below this is degenerate (rules GEO-002, QT-001). |
| `tolerances.geometry.min_area_m2` | number (m²) | `0.000001` | read | A surface with area at or below this is degenerate (rule GEO-002). |
| `tolerances.geometry.max_extent_m` | number (m) | `5000.0` | read | An element whose bounding box is longer than this in any direction is flagged by rule GEO-003. |

## `qto`

| Key | Type | Default | Status | Meaning |
| --- | --- | --- | --- | --- |
| `qto.group_by` | list of text | `[type, storey]` | read | How takeoff rows are grouped. Any non-empty combination of `type`, `storey`, `classification`, `material`, `predefined_type`, `type_name`. Other values are errors. |
| `qto.unit_rates_file` | text (path) | none | read | Unit-rate CSV for pricing. Format in [../examples/README.md](../examples/README.md#unit-rates-ratescsv). |
| `qto.currency` | text | `EUR` | read | Currency used for a rate that has no currency of its own. |
| `qto.sample_count` | whole number | `5` | read | How many elements with quantities are listed as hand-check samples. |

## `clash`

| Key | Type | Default | Status | Meaning |
| --- | --- | --- | --- | --- |
| `clash.enabled` | boolean | `false` | read | Turns clash detection on. While clash detection is not implemented, enabling it returns an explicit "not implemented" message and no clashes. |
| `clash.set_a_filter` | `ElementFilter` | empty | declared | Elements of the first model to test. Not read yet. |
| `clash.set_b_filter` | `ElementFilter` | empty | declared | Elements of the second model to test. Not read yet. |

### `ElementFilter` (used by `clash.set_a_filter`, `clash.set_b_filter` and rule scopes)

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `ifc_type` | text | none | IFC type or supertype, for example `IfcPipeSegment`. |
| `name_pattern` | regular expression | none | Element name must match. |
| `storey` | text | none | Exact storey name. |
| `property_equals` | map of `Pset.Property` to text | `{}` | Each listed property must have exactly this value. |

## `output`

| Key | Type | Default | Status | Meaning |
| --- | --- | --- | --- | --- |
| `output.language` | `en` or `es` | `en` | read | Language of `report.html` and of BCF topic titles. |
| `output.include_3d_snapshot` | boolean | `true` | declared | Whether BCF viewpoints include a snapshot. No code reads this key yet. |
| `output.bcf_version` | `"2.1"` or `"3.0"` | `"2.1"` | read | BCF version written to `issues.bcfzip`. Quoted in the file so it stays text. |
| `output.bcf_author` | text | `verimodel@local` | read | Author recorded on every BCF topic. |

## `checks`

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `checks.rules` | map of rule id to boolean | `{}` | `false` switches one rule off, `true` switches it on. Overrides the category setting. Missing ids stay enabled. |
| `checks.categories` | map of category to boolean | `{}` | `false` switches a whole category off. Missing categories stay enabled. |
| `checks.unique_name_within` | list of `type` or `storey` | `[]` | Scope of rule ID-003 (unique names). Empty means `[type, storey]`. `[type]` makes names unique across the whole model. |

Resolution order for a rule: its id in `checks.rules` if present; otherwise its
category in `checks.categories`; otherwise enabled. A rule can also be disabled
in its own rule pack (`enabled: false`).

Category names: `identity`, `spatial`, `properties`, `geometry`,
`classification`, `naming`, `sanity`, `schema`, and `ids` (for IDS-derived
rules, once IDS checks run).

## `naming`

| Key | Type | Default | Status | Meaning |
| --- | --- | --- | --- | --- |
| `naming.patterns` | map of IFC type to regular expression | `{}` | read | Element names of that type, or of any of its subtypes, must match the expression. Read by rule NAM-001. Empty means the rule is skipped. |

## `classification`

| Key | Type | Default | Status | Meaning |
| --- | --- | --- | --- | --- |
| `classification.required` | boolean | `false` | read | `true` turns on rule CL-001: every element in scope must carry a classification reference from `project.classification_system`. |
| `classification.applies_to` | list of text | `[]` | used | Element types (or supertypes) that must be classified, e.g. `[IfcWall, IfcSlab]`. Empty means every element in the rule's scope (`IfcElement`). Read by the `classification_exists` check (rule CL-001). |
| `classification.allowed_codes` | list of text | `[]` | read | Codes accepted inline. Read by rule CL-002. |
| `classification.allowed_codes_file` | text (path) | none | read | File of accepted codes: either a CSV with a `code` column (the first line is the header), or one code per line. Lines starting with `#` are ignored. Read when the checks run. |
| `classification.code_pattern` | regular expression | none | read | Every classification code must match it (rule CL-002). Checks shape, not existence in a table. |

Rule CL-002 behaviour, depending on what is configured:

- No allowed codes and no `code_pattern`: the rule is skipped, with a
  plain-language reason. A skipped rule is never counted as passed.
- Only `code_pattern`: each code must match the pattern.
- A list (inline or from file): each code must be in the list, and must also
  match `code_pattern` when one is set.

## Minimal and full examples

Minimal, valid:

```yaml
project:
  name: "Pilot"
```

The four complete examples are in `examples/`:
`residential/project.yaml`, `office/project.yaml`,
`infrastructure/project.yaml`.
