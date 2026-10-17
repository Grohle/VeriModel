## VeriModel 1.0.0

**Your model does not pass unless it proves it passes.**

First release of VeriModel: a local-first IFC quality gate, quantity takeoff and BCF exporter, with a read-only language-model copilot.

### Highlights
- **Generic quality gate** for any IFC model (IFC2x3, IFC4, IFC4.3): identity, spatial hierarchy, properties, geometry health, classification, naming and quantity sanity checks, all defined as data (YAML rule packs) and configured in `project.yaml`.
- **IDS 1.0 support** through IfcTester: every requirement becomes a rule, every failure quotes the requirement.
- **Quantity takeoff** from base quantities with geometry fallback, configurable grouping, unit-rate costing and hand-check samples.
- **BCF 2.1 / 3.0 export** validated against the official buildingSMART schemas, plus CSV for Excel.
- **Deterministic, auditable outputs**: `report.html`, `report.json`, `failures.csv`, `qto.csv/xlsx/json`, `issues.bcfzip`, `run.json` ledger with SHA-256 hashes, `audit.log`.
- **Desktop app** (Tauri 2) with a 3-step flow, EN/ES, dark/light, metric/imperial, run history with diffs, and a demo model that loads in one click.
- **CLI** with the same engine: `check`, `qto`, `inspect`, `validate-config`, `doctor`, `ask`, `docs-to-yaml`.
- **Assistant** (optional, downloads a 0.5B model on demand, runs on CPU): finds failures, explains them, drafts `project.yaml`, reads your imported documentation. Never part of a verdict.

### Requirements
- A Python 3.9+ with `ifcopenshell` and `ifctester` (`pip install ifcopenshell ifctester`). Run `verimodel doctor` to check.

### Assets
- `VeriModel_1.0.0_x64-setup.exe` / `.msi` (Windows), `.dmg` (macOS), `.AppImage` / `.deb` (Linux)
- `verimodel-cli-*` headless command line
- `SHA256SUMS.txt`

Licence: PolyForm Noncommercial 1.0.0 (free for noncommercial use; commercial licences on request).
