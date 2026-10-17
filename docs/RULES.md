# Rules and rule packs

A **rule** says what must be true of the elements in a model. A **rule pack**
is a YAML file holding a list of rules. VeriModel ships one built-in pack,
`crates/core/rule_packs/generic.yaml`, and a project can replace it with its
own pack.

Rules are data, not code. The engine knows a fixed vocabulary of **check
kinds** (listed below). A new rule that uses an existing check kind needs only
a YAML entry. A new kind of check needs code in `crates/core/src/rules/checks.rs`.

Everything in this page is taken from `crates/core/src/rules/pack.rs`,
`crates/core/src/rules/checks.rs`, `crates/core/src/rules/engine.rs`,
`crates/core/src/config.rs` and `crates/core/src/score.rs`.

## Rule pack format

```yaml
name: my_pack            # required, text
version: 1               # optional, whole number, default 1
description: Optional text
rules:                   # required, at least one rule
  - id: ABC-001          # required, unique within the pack
    category: properties # required; used for on/off switches and grouping
    description: Walls must state their fire rating
    severity: error      # optional, default error: error | warning | info
    enabled: true        # optional, default true
    enabled_if: "$config.classification.required"   # optional, see below
    scope:               # optional; default selects non-spatial products
      ifc_types: [IfcWall]
      exclude_types: []
      include_spatial: false
      filter: {}         # optional ElementFilter, see below
    check:               # required: one check kind with its parameters
      kind: property_exists
      pset: Pset_WallCommon
      property: FireRating
    fix_hint: Fill in FireRating in the authoring tool.   # optional, but always write one
```

Parser rules (every key is checked; unknown keys are errors):

- A pack must contain at least one rule. Rule ids must be unique in the pack.
- `severity` must be `error`, `warning` or `info` (case does not matter).
- `enabled_if` must start with `$config.`.
- `scope.filter.name_pattern`, when given, must be a valid regular expression.
- A `check` with an unknown `kind` is rejected when the pack is read.

Other packs are loaded by setting `requirements.rule_pack` in `project.yaml` to
a path, relative to that file: see
[`examples/infrastructure/infrastructure_rules.yaml`](../examples/infrastructure/infrastructure_rules.yaml).

## Scope

`scope` picks the elements a rule looks at.

| Key | Default | Meaning |
| --- | --- | --- |
| `ifc_types` | `[]` | Element types, or supertypes, to include. Empty means every product. |
| `exclude_types` | `[]` | Types or supertypes to leave out. |
| `include_spatial` | `false` | Include sites, buildings, storeys, spaces and similar. Only matters when `ifc_types` is empty: a spatial element is then left out unless this is `true`. When `ifc_types` is set, the list alone decides. |
| `filter` | none | Extra conditions, below. |

`ElementFilter` (also used for clash sets):

| Key | Meaning |
| --- | --- |
| `ifc_type` | Type or supertype. |
| `name_pattern` | Regular expression the name must match. |
| `storey` | Exact storey name. |
| `property_equals` | Map of `Pset.Property` to a value that must match exactly, for example `Pset_WallCommon.IsExternal: "true"`. |

Example:

```yaml
scope:
  ifc_types: [IfcWall]
  filter:
    storey: "Level 1"
    property_equals:
      "Pset_WallCommon.IsExternal": "true"
```

## Parameters: literals and `$config.` references

A parameter can be a number, a boolean, text, or a list of text. A text value
that starts with `$config.` is replaced by the value at that path in
`project.yaml` when the rule runs:

```yaml
check: { kind: distance_from_origin, max_m: "$config.tolerances.far_from_origin_m" }
```

Paths use dots and follow the layout of `project.yaml` (see
[CONFIG.md](CONFIG.md)). A path that does not exist stops the run with an error
that names the path. Thresholds written this way are changed in one
place, and the same rule file works for every project.

## `enabled_if`

`enabled_if: "$config.<path>"` runs the rule only when that value is `true`.
A missing path counts as `false`. When the condition is false, the rule is left
out of the report entirely (it is neither passed nor failed).

```yaml
enabled_if: "$config.classification.required"
```

## Switching rules on and off

Three levels, most specific first:

1. `enabled: false` in the rule pack disables the rule for every project using that pack.
2. `checks.rules` in `project.yaml` switches one rule by id:
   `checks.rules: { ID-003: false }`.
3. `checks.categories` in `project.yaml` switches all rules of one category:
   `checks.categories: { naming: false }`.

`checks.rules` wins over `checks.categories`. A rule with neither setting is on.
Rules that `project.yaml` generates from `required_psets` take the ids
`CFG-PSET-n` and `CFG-PSET-n-m` (see [CONFIG.md](CONFIG.md)).

Categories used by the built-in pack: `identity`, `spatial`, `properties`,
`geometry`, `classification`, `naming`, `sanity`, `schema`. IDS-derived rules
use `ids`. Any text is accepted as a category name.

## Check kinds

Each entry gives the kind, its parameters (required unless marked optional),
and what counts as a failure. Element checks report one finding per failing
element. Model checks report one finding for the model as a whole.

### `unique_global_id` (per element)

No parameters. Fails every element whose GlobalId is used by another element in scope.

```yaml
check: { kind: unique_global_id }
```

### `valid_global_id` (per element)

No parameters. Fails an element whose GlobalId is not 22 characters from the IFC
base64 alphabet (`0-9 A-Z a-z _ $`) with a first character from `0` to `3`.
This is a syntax check only.

```yaml
check: { kind: valid_global_id }
```

### `unique_name` (per element)

| Parameter | Required | Meaning |
| --- | --- | --- |
| `within` | no | List of `type` and/or `storey`. Default: `checks.unique_name_within` from `project.yaml`, else `[type, storey]`. |

Fails every element that shares its name with another element with the same
keys. Elements with an empty name are not compared.

```yaml
check: { kind: unique_name, within: [type] }
```

### `contained_in_spatial_structure` (per element)

No parameters. Fails an element that has no spatial container (storey, space, site or other spatial element).

```yaml
check: { kind: contained_in_spatial_structure }
```

### `container_exists` (per element)

No parameters. Fails an element whose container is not in the model's spatial tree.

```yaml
check: { kind: container_exists }
```

### `spatial_hierarchy` (model)

| Parameter | Required | Meaning |
| --- | --- | --- |
| `require` | yes | List of groups. Each group is a list of alternative types. The spatial tree of the model must contain at least one node of each group. |

Fails once for each group that is missing. Matching is by type name in the
spatial tree, ignoring case.

```yaml
check:
  kind: spatial_hierarchy
  require:
    - [IfcSite]
    - [IfcBuilding, IfcFacility, IfcBridge]
    - [IfcBuildingStorey, IfcBridgePart]
```

### `distance_from_origin` (per element)

| Parameter | Required | Meaning |
| --- | --- | --- |
| `max_m` | yes | Largest allowed distance in metres from the origin to the bounding-box centre. Number or `$config.` reference. |

Fails an element whose bounding-box centre is farther than `max_m`. Skipped,
with a reason, when the geometry kernel is unavailable.

```yaml
check: { kind: distance_from_origin, max_m: "$config.tolerances.far_from_origin_m" }
```

### `pset_exists` (per element)

| Parameter | Required | Meaning |
| --- | --- | --- |
| `pset` | yes | Property set name. |

Fails an element that does not have the property set.

```yaml
check: { kind: pset_exists, pset: Pset_ConcreteElementGeneral }
```

### `property_exists` (per element)

| Parameter | Required | Default | Meaning |
| --- | --- | --- | --- |
| `pset` | no | any set | Property set to look in. If omitted, every property set is searched. |
| `property` | yes | | Property name. |
| `require_value` | no | `true` | If `true`, an empty value (null, empty text or empty list) fails. |
| `data_type` | no | none | `string` (also `text`, `label`), `number` (also `real`, `integer`, `float`) or `boolean` (also `bool`, `logical`). Other values are not checked. |

Fails when the property is missing, when it is empty and `require_value` is
`true`, or when its type is not the one given.

```yaml
check:
  kind: property_exists
  pset: Pset_WallCommon
  property: FireRating
  require_value: true
  data_type: string
```

### `property_matches` (per element)

| Parameter | Required | Meaning |
| --- | --- | --- |
| `pset` | no | Property set to look in. Searches every set when omitted. |
| `property` | yes | Property name. |
| `pattern` | no | Regular expression the value must match. |
| `allowed_values` | no | List of values, one of which the value must equal. |

Checks only properties that exist. A missing property is not a failure of this
kind; use `property_exists` for that. The value is compared as text.

```yaml
check:
  kind: property_matches
  pset: Pset_DoorCommon
  property: FireRating
  pattern: "^(EI|REI)[0-9]+$"
  allowed_values: [EI30, EI60, EI90]
```

### `no_empty_properties` (per element)

No parameters. Fails an element when any of its property values, in any set, is empty.
The finding lists the empty ones.

```yaml
check: { kind: no_empty_properties }
```

### `has_geometry` (per element)

No parameters. Fails an element that has no product representation.

```yaml
check: { kind: has_geometry }
```

### `non_degenerate_geometry` (per element)

| Parameter | Required | Default | Meaning |
| --- | --- | --- | --- |
| `min_volume_m3` | no | `tolerances.geometry.min_volume_m3` | Volume threshold, m³. |
| `min_area_m2` | no | `tolerances.geometry.min_area_m2` | Area threshold, m². |

Fails an element that has a representation but whose shape could not be built,
has no triangles, or has a volume at or below `min_volume_m3` **and** an area at
or below `min_area_m2`. Skipped, with a reason, without the geometry kernel.

```yaml
check:
  kind: non_degenerate_geometry
  min_volume_m3: "$config.tolerances.geometry.min_volume_m3"
  min_area_m2: "$config.tolerances.geometry.min_area_m2"
```

### `bbox_within_bounds` (per element)

| Parameter | Required | Meaning |
| --- | --- | --- |
| `max_extent_m` | yes | Largest allowed bounding-box side, metres. Number or `$config.` reference. |

Fails an element whose bounding box is longer than `max_extent_m` in any
direction. Skipped without the geometry kernel.

```yaml
check: { kind: bbox_within_bounds, max_extent_m: "$config.tolerances.geometry.max_extent_m" }
```

### `classification_exists` (per element)

| Parameter | Required | Meaning |
| --- | --- | --- |
| `system` | no | Text the classification system name must contain, ignoring case. Usually `"$config.project.classification_system"`. A value of `none` or empty means any system. |

Fails an element with no classification reference that has a code (from the
given system, if one is named).

```yaml
check: { kind: classification_exists, system: "$config.project.classification_system" }
```

### `classification_code_allowed` (per element)

No parameters. The rules come from `project.yaml`, section `classification`:
`allowed_codes`, `allowed_codes_file` and `code_pattern`. Skipped, with a
reason, when none of them is set. See [CONFIG.md](CONFIG.md#classification).

```yaml
check: { kind: classification_code_allowed }
```

### `name_matches` (per element)

| Parameter | Required | Meaning |
| --- | --- | --- |
| `pattern` | no | Regular expression every name in scope must match. |

Without `pattern`, the expression for each element is taken from
`naming.patterns` in `project.yaml`, using the first entry (in alphabetical
order of type name) whose type matches the element. Skipped, with a reason, when there is no pattern at all.

```yaml
check: { kind: name_matches, pattern: "^W-[0-9]{3}$" }
```

### `quantity_positive` (per element)

| Parameter | Required | Default | Meaning |
| --- | --- | --- | --- |
| `quantity` | yes | | `volume` (m³), `area` (m²) or `length` (m). |
| `min` | no | `0` | The value must be greater than this. Number or `$config.` reference. |

Takes the value from the base quantities in the file when present, otherwise
from the geometry (see [ASSUMPTIONS.md](ASSUMPTIONS.md)). Fails when the value
is at or below `min`. Elements without any value are not checked.

```yaml
check: { kind: quantity_positive, quantity: volume, min: "$config.tolerances.geometry.min_volume_m3" }
```

### `types_present` (model)

| Parameter | Required | Meaning |
| --- | --- | --- |
| `ifc_types` | yes | List of types, or supertypes. Each must have at least one element in the model. |

Fails once for each missing type.

```yaml
check: { kind: types_present, ifc_types: [IfcBridge] }
```

### `schema_allowed` (model)

No parameters. Fails when the file's schema is not in `project.ifc_versions`. An
empty list accepts any schema. Schemas are compared after normalisation (see [ASSUMPTIONS.md](ASSUMPTIONS.md)).

```yaml
check: { kind: schema_allowed }
```

## Adding a custom rule without touching engine code

1. Copy `crates/core/rule_packs/generic.yaml`, or start a new file with the
   format above, and give it a unique `name`.
2. Add a rule that uses one of the check kinds above.
3. Give it an `id` that is unique in the pack, a `category`, a `description`,
   a `severity`, a `scope`, a `check` and a `fix_hint`.
4. Point `requirements.rule_pack` at the file in `project.yaml`.
5. Check it with `verimodel validate-config --config project.yaml`, then run
   `verimodel check` on a model.

Example: a rule that every door has a fire rating from a standard list, with the
list kept in one place (`project.yaml`) by using `property_matches` with
`allowed_values` written inline:

```yaml
- id: DOR-001
  category: properties
  description: Doors must state a fire rating from the approved list
  severity: error
  scope: { ifc_types: [IfcDoor] }
  check:
    kind: property_matches
    pset: Pset_DoorCommon
    property: FireRating
    allowed_values: [EI30, EI60, EI90]
  fix_hint: Set Pset_DoorCommon.FireRating to one of EI30, EI60 or EI90.
```

A check kind that does not exist cannot be added this way: it needs code in
`checks.rs`.

## The built-in generic pack

`crates/core/rule_packs/generic.yaml` has 17 rules.

| Id | Category | Severity | Check kind | What it checks |
| --- | --- | --- | --- | --- |
| ID-001 | identity | error | `unique_global_id` | GlobalIds are unique |
| ID-002 | identity | error | `valid_global_id` | GlobalIds are valid |
| ID-003 | identity | warning | `unique_name` | Names unique within type and storey |
| SP-001 | spatial | error | `contained_in_spatial_structure` | Every element has a container |
| SP-002 | spatial | error | `container_exists` | Containers exist in the tree |
| SP-003 | spatial | warning | `spatial_hierarchy` | Site, building or facility, and storey or part present |
| SP-004 | spatial | warning | `distance_from_origin` | Not far from origin (`tolerances.far_from_origin_m`) |
| PR-001 | properties | warning | `no_empty_properties` | No empty property values |
| GEO-001 | geometry | warning | `has_geometry` | Physical elements have a body |
| GEO-002 | geometry | error | `non_degenerate_geometry` | Shapes are not degenerate |
| GEO-003 | geometry | warning | `bbox_within_bounds` | Elements within `tolerances.geometry.max_extent_m` |
| CL-001 | classification | error | `classification_exists` | Classified (only when `classification.required` is `true`) |
| CL-002 | classification | warning | `classification_code_allowed` | Codes are allowed |
| NAM-001 | naming | warning | `name_matches` | Names follow `naming.patterns` |
| QT-001 | sanity | warning | `quantity_positive` (volume) | Positive volume |
| QT-002 | sanity | warning | `quantity_positive` (area) | Positive area |
| SCH-001 | schema | error | `schema_allowed` | Schema is accepted by the project |

## Verdict and score

Quoted from `crates/core/src/score.rs`.

Verdict rule (shown in the report):

> FAIL = at least one ERROR. PASS WITH WARNINGS = no errors, at least one WARNING. PASS = no errors and no warnings. Score = 100 × (1 − (errors + 0.25 × warnings) / element checks). Skipped rules are never counted as passed.

In detail:

- **Verdict.** `FAIL` if any finding has severity error; otherwise `PASS_WITH_WARNINGS` if any has severity warning; otherwise `PASS`. There is no verdict when no rules ran.
- **Score.** `100 × (1 − (errors + 0.25 × warnings) / element_checks)`, rounded to a whole number and clamped to 0 to 100. `element_checks` is the sum, over rules that did not skip, of the number of elements each rule evaluated (a model-level rule counts as one). INFO findings do not change the score.
- **Edge case.** With no element checks at all, the score is 0 if there is an error and 100 otherwise.
- **Rule statuses.** Pass, Fail, Skipped (the rule could not run, with a reason; never counted as passed), and Not applicable (no element was in scope).
