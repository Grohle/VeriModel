// Single gateway to the Rust engine. Every call goes through Tauri's invoke
// when running inside the desktop app and falls back to JSON in mock/ when
// window.__TAURI__ is undefined, so the whole UI runs in a plain browser for
// development and screenshots.

const tauri = () => window.__TAURI__;
export const isDesktop = () => Boolean(tauri() && tauri().core && tauri().core.invoke);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const cache = new Map();

async function mockJson(name) {
  if (!cache.has(name)) {
    cache.set(name, fetch(`mock/${name}.json`).then((r) => {
      if (!r.ok) throw new Error(`mock/${name}.json: ${r.status}`);
      return r.json();
    }));
  }
  return cache.get(name);
}
const fresh = async (name) => structuredClone(await mockJson(name));

// Errors from the engine arrive as { code, message, hint }. Anything else is
// wrapped so the UI never shows a stack trace.
export function normaliseError(e) {
  if (e && typeof e === 'object' && typeof e.message === 'string' && ('code' in e || 'hint' in e)) {
    return { code: e.code || 'error', message: e.message, hint: e.hint || '' };
  }
  if (typeof e === 'string') return { code: 'error', message: e, hint: '' };
  return { code: 'error', message: String(e && e.message ? e.message : e), hint: '' };
}

async function invoke(cmd, args) {
  try {
    return await tauri().core.invoke(cmd, args);
  } catch (e) {
    throw normaliseError(e);
  }
}

// ---------------------------------------------------------------- commands

export async function inspectFile(path) {
  if (isDesktop()) return invoke('inspect_file', { path });
  await sleep(450);
  if (/broken|corrupt/i.test(path)) {
    throw {
      code: 'ifc_parse_error',
      message: 'This file could not be read as an IFC model. It stops in the middle of the DATA section (line 1,204).',
      hint: 'Re-export the model from your authoring tool as IFC4 or IFC2x3 and check the export finished without errors. If the file was copied or downloaded, check it is not truncated.',
    };
  }
  return fresh('file_card');
}

export async function runCheck(args) {
  if (isDesktop()) return invoke('run_check', args);
  await sleep(2400);
  const [report, runs, extra] = await Promise.all([fresh('report'), fresh('runs'), fresh('run_result')]);
  return { report, ledger: runs[0], files: extra.files, bcf: extra.bcf, out_dir: extra.out_dir };
}

export async function listRuns(root = null) {
  if (isDesktop()) return invoke('list_runs', { root });
  await sleep(120);
  return fresh('runs');
}

const MOCK_DEFAULT_YAML = `# VeriModel project.yaml (defaults)
project:
  name: Generic Project
  classification_system: none
requirements:
  rule_pack: generic
tolerances:
  far_from_origin_m: 10000
  geometry:
    min_volume_m3: 0.000001
classification:
  required: false
qto:
  group_by: [type, storey]
  currency: EUR
output:
  language: en
`;

export async function defaultConfigYaml() {
  if (isDesktop()) return invoke('default_config_yaml');
  return MOCK_DEFAULT_YAML;
}

export async function validateConfigYaml(yaml) {
  if (isDesktop()) return invoke('validate_config_yaml', { yaml });
  if (/group_by:\s*\[\s*\]/.test(yaml)) return { ok: false, error: 'qto.group_by must contain at least one key' };
  return { ok: true, error: null };
}

export async function listExamples() {
  if (isDesktop()) return invoke('list_examples');
  return fresh('examples');
}

export async function getElement(globalId) {
  if (isDesktop()) return invoke('get_element', { global_id: globalId });
  const all = await mockJson('elements');
  const el = all[globalId];
  if (!el) throw { code: 'element_not_found', message: 'This element is not in the loaded model.', hint: 'Run the check again to refresh the model data.' };
  return el;
}

// Returns true when the OS was asked to open the path.
export async function openPath(path) {
  if (isDesktop()) { await invoke('open_path', { path }); return true; }
  return false;
}

// PROPOSED command (not in the original contract): read_text_file({ path }) -> string.
// Needed to load a dropped project.yaml or a sample's project.yaml.
export async function readTextFile(path) {
  if (isDesktop()) return invoke('read_text_file', { path });
  const f = browserFiles.get(path);
  if (f) return f.text();
  await sleep(80);
  return '# project.yaml of the selected sample\nclassification:\n  required: true\nproject:\n  classification_system: Uniclass\n';
}

export async function saveTextFile(contents, defaultName) {
  if (isDesktop()) {
    const dlg = tauri().dialog;
    const path = dlg && dlg.save ? await dlg.save({ defaultPath: defaultName }) : null;
    if (!path) return null;
    await invoke('save_text_file', { path, contents });
    return path;
  }
  const url = URL.createObjectURL(new Blob([contents], { type: 'text/yaml' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = defaultName;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
  return defaultName;
}

// ------------------------------------------------------------ file pickers

const browserFiles = new Map(); // name -> File (browser fallback only)

const KINDS = {
  ifc: { name: 'IFC', extensions: ['ifc', 'ifczip'] },
  ids: { name: 'IDS', extensions: ['ids', 'xml'] },
  yaml: { name: 'YAML', extensions: ['yaml', 'yml'] },
  csv: { name: 'CSV', extensions: ['csv'] },
};

export async function pickFile(kind) {
  const filter = KINDS[kind];
  if (isDesktop() && tauri().dialog) {
    const r = await tauri().dialog.open({ multiple: false, filters: [filter] });
    if (!r) return null;
    return typeof r === 'string' ? r : r.path || null;
  }
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = filter.extensions.map((e) => `.${e}`).join(',');
    input.addEventListener('change', () => {
      const f = input.files && input.files[0];
      if (!f) return resolve(null);
      browserFiles.set(f.name, f);
      resolve(f.name);
    });
    input.addEventListener('cancel', () => resolve(null));
    input.click();
  });
}

// handlers: { enter(), leave(), drop(paths: string[]) }
export async function onDragDrop(handlers) {
  if (isDesktop() && tauri().event) {
    const ev = tauri().event;
    await ev.listen('tauri://drag-enter', () => handlers.enter());
    await ev.listen('tauri://drag-leave', () => handlers.leave());
    await ev.listen('tauri://drag-drop', (e) => {
      handlers.leave();
      const paths = (e.payload && e.payload.paths) || [];
      if (paths.length) handlers.drop(paths);
    });
    return;
  }
  let depth = 0;
  const hasFiles = (e) => e.dataTransfer && Array.from(e.dataTransfer.types || []).includes('Files');
  window.addEventListener('dragenter', (e) => { if (!hasFiles(e)) return; e.preventDefault(); if (depth++ === 0) handlers.enter(); });
  window.addEventListener('dragover', (e) => { if (hasFiles(e)) e.preventDefault(); });
  window.addEventListener('dragleave', (e) => { if (!hasFiles(e)) return; if (--depth <= 0) { depth = 0; handlers.leave(); } });
  window.addEventListener('drop', (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth = 0;
    handlers.leave();
    const files = Array.from(e.dataTransfer.files || []);
    files.forEach((f) => browserFiles.set(f.name, f));
    if (files.length) handlers.drop(files.map((f) => f.name));
  });
}
