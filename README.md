# VeriModel

> **Your model does not pass unless it proves it passes.**

VeriModel is a local-first desktop app: **IFC QA Gate + Quantity Takeoff (QTO) + BCF Export**.

It takes ANY IFC model from ANY BIM project and answers, with proof:

1. Is this model compliant with the project's requirements?
2. How much is in it (quantities)?
3. What is wrong, exactly where, in a format a coordinator can open in any BCF tool?

Not a bot that clicks. Not a flow builder. A quality gate a model must cross with evidence: audit trail, policy / IDS gate, deterministic checks, reproducible outputs.

## Status

Scaffold only (README + LICENSE + .gitignore + directory layout). No engine yet.

## Why VeriModel

- **Verifiable, not vibes:** every number has evidence one click away. Deterministic code decides pass/fail, never an LLM.
- **Generic by design:** any building / infrastructure type, any discipline, any IFC schema (IFC2x3, IFC4, IFC4.3), any units, any classification system, any language. All project-specific values come from `project.yaml` + `.ids`, never from hardcoded lists in code.
- **Local-first:** no accounts, no cloud, no telemetry. All data stays on the machine.
- **Standards-based:** IFC + buildingSMART IDS (Information Delivery Specification) + BCF (BIM Collaboration Format).

## Planned architecture

Rust engine + HTML interface via **Tauri v2**.

```
verimodel/
  crates/
    core/         # Pure Rust: rule engine, QTO aggregation, BCF writer, audit, report, config
    ifc_reader/   # IFC reading bridge: calls IfcOpenShell for parsing + geometry + IDS, returns facts to core
    cli/          # Rust CLI, shares crates/core with the desktop app
  src-tauri/      # Tauri v2 app: exposes core via Tauri commands, file dialogs, bundling
  frontend/       # HTML/CSS/JS UI — presentation only, no business logic
  examples/       # 3 example project.yaml + sample IDS + sample rates.csv for 3 different project types
  tests/          # Rust integration tests + fixtures
```

Division of responsibilities:

- **Rust `core` does ALL deterministic logic:** rule engine, QTO, BCF, audit ledger, report, config validation.
- **`ifc_reader` does ONLY IFC reading:** it returns facts, `core` decides pass/fail.
- **Frontend does ONLY presentation:** file pickers, forms, tables, filters, downloads.

Why hybrid: there is no mature pure-Rust equivalent to IfcOpenShell for geometry + IDS + quantities today. Using IfcOpenShell as the reader and Rust as the brain gives a real Rust engine without reinventing the geometry kernel.

## Planned UI — 3 steps

1. **Load** — drag & drop IFC + optional IDS + `project.yaml`. File card: schema, element count, units, disciplines, hash.
2. **Review** — overall score + verdict (PASS / PASS WITH WARNINGS / FAIL), per-rule pass/fail, filterable failures (severity / rule / storey / type), QTO tables, element detail on click.
3. **Export** — `report.html`, `failures.csv`, `qto.csv` + `qto.xlsx`, `issues.bcfzip`, `run.json`, `project.yaml`. Plus run history with diff vs previous run.

Design: intuitive, one primary action per screen, evidence first, dark + light themes, EN/ES, metric/imperial.

## Planned outputs (per run)

- `report.html` — human-readable summary + per-rule table + QTO + failure list
- `report.json` — machine-readable, same data
- `failures.csv` — one row per element per rule
- `qto.csv` + `qto.xlsx` — quantities (+ cost if rates provided)
- `issues.bcfzip` — BCF 2.1/3.0, opens in any free BCF viewer
- `run.json` — run ledger: input SHA256, IDS SHA256, config SHA256, version, duration, counts, scores
- `audit.log` — append-only log

Determinism: same IFC + same IDS + same config = same `report.json` / `failures.csv` / `qto.csv` byte-for-byte (except timestamps in `run.json`).

## Planned CLI

```
verimodel check --ifc model.ifc --ids spec.ids --config project.yaml --out ./out
verimodel qto --ifc model.ifc --config project.yaml --out ./out
verimodel clash --ifc-a model_a.ifc --ifc-b model_b.ifc --config project.yaml --out ./out
verimodel validate-config --config project.yaml
```

CLI and desktop app share the same Rust core — never two implementations.

## Roadmap

- v0.1: scaffold + core rule engine + one end-to-end check (duplicate GlobalId) + one QTO table + HTML report on a sample IFC
- v0.2: full generic check categories + IDS + BCF + 3-step UI
- v0.3: clash (phase 2), revision diff, run history

## License

MIT. See [LICENSE](LICENSE).

Developed by **Alberto Miranda**.

---

## Español

VeriModel es una app de escritorio local: **puerta de calidad IFC + mediciones (QTO) + exportación BCF**.

Toma CUALQUIER modelo IFC de CUALQUIER proyecto BIM y responde, con pruebas: ¿cumple los requisitos del proyecto?, ¿cuánto hay?, ¿qué está mal y dónde, en un formato que un coordinador puede abrir en cualquier visor BCF?

No es un bot que hace clics ni un constructor de flujos. Es la puerta que un modelo tiene que cruzar con evidencia.

Motor Rust + interfaz HTML (Tauri v2), local-first, sin cuentas ni nube. Genérica por diseño: cualquier tipo de proyecto, disciplina, esquema IFC, unidades o sistema de clasificación — todo lo específico del proyecto viene de `project.yaml` + `.ids`, nunca de listas hardcodeadas en el código.
