# Contributing to VeriModel

VeriModel is a local-first IFC quality gate, quantity takeoff and BCF exporter,
source-available under the PolyForm Noncommercial License 1.0.0.
Read this before you open a pull request.

## Before touching anything

```sh
pip install ifcopenshell ifctester       # the IFC reader bridge
cargo test --workspace                   # unit + end-to-end tests (real IFC fixtures)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo tauri dev                          # the desktop app
```

## Where things live

- `README.md`: what VeriModel is, how to run it, architecture, demo script.
- `docs/CONFIG.md`, `docs/RULES.md`, `docs/IDS.md`, `docs/ASSUMPTIONS.md`: reference.
- `crates/core`: every decision (rules, score, QTO, BCF, reports). No IFC dependency.
- `crates/ifc_reader`: the only code that touches IFC (IfcOpenShell bridge). Facts only.
- `crates/assistant`: the read-only language-model copilot. Never part of a verdict.
- `frontend/`: presentation only. `src-tauri/` and `crates/cli/`: thin shells over core.
- `examples/`, `tests/fixtures/`: sample configurations and public IFC files.

## Work rules

1. Nothing project-specific in Rust: element types, property names, storeys, tolerances and classification come from `project.yaml`, `.ids` or a rule pack. A new check is a new YAML rule, or a new generic check kind with tests.
2. No change ships without the suite green, including the end-to-end tests on the fixtures.
3. Every fixed bug carries a regression test, verified to fail without the fix.
4. Outputs stay deterministic: `report.json`, `failures.csv`, `qto.*` and `report.html` must be byte-identical for the same inputs. Timestamps live only in `run.json` and `audit.log`.
5. The verdict is never produced or influenced by the assistant model.
6. Code, identifiers and comments in English. UI strings in both `en` and `es`, through the i18n files, never hardcoded.
7. A check that cannot run is reported as SKIPPED with a reason, never as PASS.
8. Do not add dependencies without a line in the README's "Dependencies and licenses" table.

## Licence

By contributing you agree that your contribution is licensed under the project
licence (PolyForm Noncommercial 1.0.0) and that the copyright holder may license
the project, including your contribution, under other terms.
