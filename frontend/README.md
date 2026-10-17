# VeriModel frontend

Vanilla HTML + CSS + ES modules. No npm, no bundler, no framework, no CDN, no web fonts. The folder is served as-is.

## Run in a browser (development, mock data)

ES modules and `fetch` do not work over `file://`, so serve the folder with any static server:

```sh
cd frontend
python3 -m http.server 8765
# open http://localhost:8765
```

When `window.__TAURI__` is undefined, `js/api.js` answers every command from `mock/*.json`
(`report.json`, `file_card.json`, `runs.json`, plus `examples.json`, `run_result.json`, `elements.json`).
Drop or pick any file to see the mock file card; a file whose name contains `broken` shows the parse-error card.
Use "Try with a sample" for the quickest walk-through.

## How Tauri loads it

`src-tauri/tauri.conf.json` sets `frontendDist` to `../frontend` and `withGlobalTauri: true`, so
`index.html` is served directly and `window.__TAURI__` is available. `js/api.js` is the only file that
calls `core.invoke`, `dialog.open/save` and `event.listen` (drag and drop).

Commands used: `inspect_file`, `run_check`, `list_runs`, `default_config_yaml`, `validate_config_yaml`,
`list_examples`, `open_path`, `save_text_file`, `get_element`, `read_text_file`, `app_info`, and the assistant commands
`assistant_status`, `assistant_download_model` (emits the `assistant://download-progress` event `{ done, total }`),
`assistant_remove_model`, `assistant_ask({ task })`, `assistant_import_docs({ paths })`, `assistant_knowledge_yaml`.

## Layout

- `index.html`, `styles.css` (tokens on `:root[data-theme]`), `app.js` (boot, routing, run flow)
- `js/` modules: `api`, `i18n`, `state`, `units`, `yaml`, `settings`, `view-load`, `view-review`, `view-export`, `detail`, `table`, `dom`, `format`
- `i18n/en.json`, `i18n/es.json`: every user-facing string, same keys. Plurals use `key_one` / `key_other`.
- Preferences persisted in `localStorage`: `vm.theme`, `vm.lang`, `vm.units`.

## Assistant panel (`js/assistant.js`)

A right-hand drawer (420 px, Escape closes) opened from the header. In a browser the answers are canned
(`mock/assistant_status.json` plus per-kind answers in `js/api.js`): a question with "level 9" shows the
dropped-value note, "[raw]" shows the unstructured-answer note, "invalid" in a settings question shows the
Invalid badge. The panel never decides pass/fail; it only proposes filters, text and YAML that the user applies
with a button. The demo example (`id: demo`) is always the first sample and has a "Load the demo" button next to the drop zone.
