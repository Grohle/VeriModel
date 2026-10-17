// Single mutable application state + persisted preferences.
// Views read from `state` and are re-rendered by app.js; cross-view calls go
// through `actions` (filled by app.js) to avoid circular imports.

export const store = {
  get(key, fallback) {
    try { return localStorage.getItem(key) ?? fallback; } catch { return fallback; }
  },
  set(key, value) {
    try { localStorage.setItem(key, value); } catch { /* storage unavailable: preference lasts this session only */ }
  },
};

const prefersDark = () => {
  try { return window.matchMedia('(prefers-color-scheme: dark)').matches; } catch { return false; }
};

export const state = {
  step: 1,
  theme: store.get('vm.theme', prefersDark() ? 'dark' : 'light'),
  lang: store.get('vm.lang', (navigator.language || 'en').toLowerCase().startsWith('es') ? 'es' : 'en'),
  units: store.get('vm.units', 'metric'),

  examples: [],
  file: null,            // FileCard from inspect_file
  inspecting: false,
  idsPath: null,
  configPath: null,      // path of a dropped/picked project.yaml
  configRaw: null,       // its text; when set the form is bypassed
  settings: null,        // filled by app.js with defaultSettings()
  configError: null,     // plain-language problem found by validation
  loadError: null,       // {code, message, hint}

  running: false,
  runStage: '',
  result: null,          // { report, ledger, files, bcf, out_dir }
  runs: [],
  runsError: null,

  review: {
    tab: 'checks',
    filters: { sev: { ERROR: true, WARNING: true, INFO: true }, rule: '', storey: '', type: '', q: '' },
    page: { checks: 1, failures: 1, quantities: 1 },
    pageSize: 50,
  },
  detail: null,          // { finding, element, status }
};

export const actions = {};
