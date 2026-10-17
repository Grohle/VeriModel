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

// --------------------------------------------------------------- assistant
// Optional local copilot. It is read-only: it never produces a verdict, a
// score or a quantity. In a plain browser the answers below are canned so the
// panel can be demoed and screenshotted.

export async function appInfo() {
  if (isDesktop()) return invoke('app_info');
  return fresh('app_info');
}

let mockInstalled = true;
let mockLoaded = false;

export async function assistantStatus() {
  if (isDesktop()) return invoke('assistant_status');
  const s = await fresh('assistant_status');
  s.installed = mockInstalled;
  s.loaded = mockInstalled && mockLoaded;
  return s;
}

// onProgress({ done, total }) is called while the model downloads.
export async function assistantDownloadModel(onProgress) {
  if (isDesktop()) {
    const ev = tauri().event;
    const unlisten = ev && ev.listen ? await ev.listen('assistant://download-progress', (e) => onProgress(e.payload)) : null;
    try { return await invoke('assistant_download_model'); } finally { if (unlisten) unlisten(); }
  }
  const total = 491400032;
  for (let i = 1; i <= 20; i++) { await sleep(130); onProgress({ done: Math.round((total * i) / 20), total }); }
  mockInstalled = true;
  mockLoaded = false;
  return assistantStatus();
}

export async function assistantRemoveModel() {
  if (isDesktop()) return invoke('assistant_remove_model');
  await sleep(200);
  mockInstalled = false;
  mockLoaded = false;
  return assistantStatus();
}

// task: the JSON of the Rust `Task` enum, tagged by `kind`.
export async function assistantAsk(task) {
  if (isDesktop()) return invoke('assistant_ask', { task });
  await sleep(1100);
  mockLoaded = true;
  return mockAnswer(task);
}

export async function assistantKnowledgeYaml() {
  if (isDesktop()) return invoke('assistant_knowledge_yaml');
  return mockKnowledgeYaml;
}

const DOC_EXTENSIONS = ['md', 'markdown', 'txt', 'yaml', 'yml', 'json', 'docx', 'pdf'];

// Returns an array of paths (files, or one folder) or null when cancelled.
export async function pickDocs({ directory = false } = {}) {
  if (isDesktop() && tauri().dialog) {
    const r = await tauri().dialog.open(directory
      ? { multiple: true, directory: true }
      : { multiple: true, filters: [{ name: 'Documents', extensions: DOC_EXTENSIONS }] });
    if (!r) return null;
    const list = (Array.isArray(r) ? r : [r]).map((x) => (typeof x === 'string' ? x : x.path)).filter(Boolean);
    return list.length ? list : null;
  }
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.multiple = true;
    if (directory) input.webkitdirectory = true; else input.accept = DOC_EXTENSIONS.map((e) => `.${e}`).join(',');
    input.addEventListener('change', () => {
      const files = Array.from(input.files || []);
      resolve(files.length ? (directory ? [files[0].webkitRelativePath.split('/')[0] || 'folder'] : files.map((f) => f.name)) : null);
    });
    input.addEventListener('cancel', () => resolve(null));
    input.click();
  });
}

let mockKnowledgeYaml = 'chunks: []\n';

export async function assistantImportDocs(paths) {
  if (isDesktop()) return invoke('assistant_import_docs', { paths });
  await sleep(700);
  const ok = paths.filter((p) => DOC_EXTENSIONS.includes((/\.([A-Za-z0-9]+)$/.exec(p) || [])[1]?.toLowerCase()) || !/\.[A-Za-z0-9]+$/.test(p));
  const skipped = paths.filter((p) => !ok.includes(p));
  const chunks = ok.length * 14;
  mockKnowledgeYaml = `chunks:\n${ok.map((p) => `  - source: ${p}\n    title: "${p} \u203a Overview"\n    text: "(imported text of ${p})"\n    keywords: []\n`).join('')}`;
  return { files_read: ok, skipped, chunks_added: chunks, knowledge_yaml: mockKnowledgeYaml };
}

// Canned answers for the browser fallback, one per task kind. Type "[raw]" in a
// question to see the "not structured" note.
function mockAnswer(task) {
  const q = String(task.question || '');
  if (/\[raw\]/i.test(q)) {
    return { kind: 'text', text: 'Probably the wall fire ratings; I could not produce a clean filter for that.', sources: [], structured: false };
  }
  switch (task.kind) {
    case 'filter': {
      const ctx = task.context || {};
      const storeys = ctx.storeys || [];
      const asked = /level\s*(\d+)/i.exec(q);
      const named = storeys.find((s) => q.toLowerCase().includes(String(s).toLowerCase()));
      const storey = named || (asked ? storeys.find((s) => s.toLowerCase() === `level ${asked[1]}`) : null) || null;
      const dropped = asked && !storey ? [`Level ${asked[1]}`] : [];
      const wantsWarnings = /warning/i.test(q);
      const filter = {
        severity: wantsWarnings ? ['WARNING'] : ['ERROR'],
        rule_id: null,
        storey: storey || (dropped.length ? null : storeys[0] || null),
        ifc_type: null,
        search: null,
        explanation: `Shows ${wantsWarnings ? 'warnings' : 'errors'}${storey || !dropped.length ? ` on ${storey || storeys[0] || 'every storey'}` : ''}. Press Apply filter to see them in the Failures table.`,
        dropped,
      };
      return { kind: 'filter', filter };
    }
    case 'explain': {
      const f = task.finding || {};
      const where = f.name ? `${f.name}${f.global_id ? ` (${f.global_id})` : ''}` : (f.global_id || 'the model');
      return { kind: 'explanation', text: [`${where}${f.storey ? ` on ${f.storey}` : ''} failed rule ${f.rule_id}.`, f.message, f.fix_hint ? `To fix it: ${f.fix_hint}` : ''].filter(Boolean).map((x) => (/[.!?]$/.test(x) ? x : `${x}.`)).join(' ') };
    }
    case 'config_help': {
      if (/\binvalid\b/i.test(q)) {
        return { kind: 'config_snippet', yaml: 'naming:\n  patterns:\n    IfcWall: [unclosed\n', valid: false, validation_error: 'naming.patterns.IfcWall: expected a regular expression string', explanation: 'This is deliberately broken to show the validity badge.' };
      }
      return {
        kind: 'config_snippet',
        yaml: "naming:\n  patterns:\n    IfcWall: '^WL-[0-9]{3}$'\n    IfcDoor: '^D-[0-9]{3}$'\nrequirements:\n  required_psets:\n    - pset: Pset_WallCommon\n      applies_to: [IfcWall]\n      properties: [FireRating, IsExternal]\n",
        valid: true,
        validation_error: null,
        explanation: 'Walls and doors must follow the naming pattern, and every wall needs FireRating and IsExternal. Paste it into your project settings.',
      };
    }
    case 'draft_rules':
      return { kind: 'config_snippet', yaml: 'requirements:\n  required_psets: []\n', valid: true, validation_error: null, explanation: 'No requirements found in the text.' };
    default:
      return {
        kind: 'text',
        text: /CL-001/i.test(q)
          ? 'CL-001 checks that every element carries a classification reference from the configured system (Uniclass in this project). Elements without one are reported as errors.'
          : 'VeriModel checks the model against rules and IDS requirements, then produces a score, a quantity takeoff and BCF issues. The assistant only helps you read them.',
        sources: ['docs/RULES.md', 'docs/CONFIG.md'],
        structured: true,
      };
  }
}
