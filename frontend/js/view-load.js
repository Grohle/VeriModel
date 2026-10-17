// Step 1: load a model (drop / browse / sample), inspect it, tune settings, run.
import { h, clear, $, setTip } from './dom.js';
import { t, tn } from './i18n.js';
import { state, actions } from './state.js';
import * as api from './api.js';
import { bytes, nf, basename, extOf } from './format.js';
import { renderSettings, isDirty, blockingProblem, localProblems } from './settings.js';

const cap = (s) => (s ? s.charAt(0).toUpperCase() + s.slice(1).toLowerCase() : s);
const tOr = (key, fallback) => { const v = t(key); return v === key ? fallback : v; };

// ------------------------------------------------------------ data actions

export async function loadIfc(path) {
  state.inspecting = true;
  state.loadError = null;
  state.file = null;
  renderLoad();
  try {
    state.file = await api.inspectFile(path);
  } catch (e) {
    state.loadError = e;
    state.file = null;
  } finally {
    state.inspecting = false;
    renderLoad();
  }
}

export async function attachConfig(path) {
  try {
    state.configRaw = await api.readTextFile(path);
    state.configPath = path;
    state.configError = null;
  } catch (e) {
    actions.toast(t('load.config.unreadable', { name: basename(path) }), 'error');
    return;
  }
  renderSettingsPanel();
  actions.settingsChanged();
}

export function clearConfigFile() {
  state.configRaw = null;
  state.configPath = null;
  state.configError = null;
  renderSettingsPanel();
  actions.settingsChanged();
}

export function attachIds(path) {
  state.idsPath = path;
  renderLoad();
}

export async function useExample(ex) {
  state.idsPath = ex.ids_path || null;
  state.configRaw = null;
  state.configPath = null;
  await loadIfc(ex.ifc_path);
  if (ex.config_path) await attachConfig(ex.config_path);
  else { renderSettingsPanel(); actions.settingsChanged(); }
  renderLoad();
}

// Classify dropped paths by extension and route them.
export async function handleDropped(paths) {
  let ifc = null;
  for (const p of paths) {
    const ext = extOf(p);
    if (ext === 'ifc' || ext === 'ifczip') { if (!ifc) ifc = p; else actions.toast(t('load.drop.extra_ifc', { name: basename(p) }), 'info'); }
    else if (ext === 'ids' || ext === 'xml') attachIds(p);
    else if (ext === 'yaml' || ext === 'yml') await attachConfig(p);
    else actions.toast(t('load.drop.unsupported', { name: basename(p) }), 'error');
  }
  if (ifc) await loadIfc(ifc);
}

// ---------------------------------------------------------------- rendering

export function renderSettingsPanel() {
  renderSettings($('#settings-body'));
  updateSettingsBadge();
}

export function updateSettingsBadge() {
  const badge = $('#settings-badge');
  const problems = state.configRaw != null ? 0 : localProblems(state.settings).length;
  if (problems || state.configError) { badge.textContent = t('settings.badge.problem'); badge.className = 'badge-count is-problem'; badge.hidden = false; }
  else if (state.configRaw != null) { badge.textContent = t('settings.badge.file'); badge.className = 'badge-count'; badge.hidden = false; }
  else if (isDirty(state.settings)) { badge.textContent = t('settings.badge.edited'); badge.className = 'badge-count'; badge.hidden = false; }
  else badge.hidden = true;
}

const demoExample = () => state.examples.find((x) => x.id === 'demo') || null;

export function renderSampleList() {
  const host = $('#sample-list');
  clear(host);
  state.examples.forEach((ex) => {
    const isDemo = ex.id === 'demo';
    const meta = h('p', { class: 'sample-meta' },
      isDemo ? h('span', { class: 'chip chip-accent' }, t('load.samples.start_here')) : null,
      h('span', { class: 'chip' }, ex.ids_path ? t('load.samples.with_ids') : t('load.samples.no_ids')),
      h('span', { class: 'chip' }, ex.config_path ? t('load.samples.with_config') : t('load.samples.default_config')));
    const btn = h('button', { type: 'button', class: 'btn btn-secondary', disabled: state.inspecting || state.running, 'aria-label': t('load.samples.use_named', { name: isDemo ? t('load.demo.card_name') : ex.name }), onclick: () => useExample(ex) }, t('load.samples.use'));
    host.appendChild(isDemo
      ? h('article', { class: 'sample is-demo' }, h('div', { class: 'sample-body' }, h('h3', null, t('load.demo.card_name')), h('p', null, t('load.demo.card_desc')), meta), btn)
      : h('article', { class: 'sample' }, h('h3', null, ex.name), h('p', null, ex.description), meta, btn));
  });
}

function renderAttachments() {
  const set = (idName, btnClear, name) => {
    const pill = $(idName);
    pill.hidden = !name;
    pill.textContent = name ? basename(name) : '';
    if (name) setTip(pill, name);
    $(btnClear).hidden = !name;
  };
  set('#ids-name', '#btn-clear-ids', state.idsPath);
  set('#yaml-name', '#btn-clear-yaml', state.configRaw != null ? (state.configPath || 'project.yaml') : null);
}

function renderErrorCard() {
  const host = $('#load-error');
  const e = state.loadError;
  host.hidden = !e;
  clear(host);
  if (!e) return;
  host.append(
    h('div', { class: 'notice-body' },
      h('h2', null, t('error.title')),
      h('p', { class: 'error-message' }, e.message),
      h('p', { class: 'error-hint' }, h('strong', null, t('error.check')), ' ', e.hint || t('error.generic_hint'))),
    h('button', { type: 'button', class: 'btn btn-secondary', onclick: () => { state.loadError = null; renderLoad(); } }, t('common.dismiss')));
}

function chipList(items, className = '') {
  return h('ul', { class: `chips ${className}` }, items.map((c) => h('li', { class: 'chip' }, c)));
}

function renderFileCard() {
  const host = $('#file-section');
  clear(host);
  host.hidden = !state.file && !state.inspecting;
  if (state.inspecting) {
    host.appendChild(h('div', { class: 'file-card is-loading', role: 'status' }, h('div', { class: 'skeleton w60' }), h('div', { class: 'skeleton w40' }), h('p', { class: 'muted' }, t('load.reading'))));
    return;
  }
  const f = state.file;
  if (!f) return;
  const fact = (labelNode, valueNode) => h('div', { class: 'fact' }, h('dt', null, labelNode), h('dd', null, valueNode));
  const sha = h('span', { class: 'mono', tip: f.sha256 }, f.sha256_short);
  const types = Object.entries(f.type_counts || {}).sort((a, b) => b[1] - a[1]);
  host.appendChild(h('section', { class: 'file-card', 'aria-labelledby': 'file-card-title' },
    h('div', { class: 'file-card-head' },
      h('div', null, h('h2', { id: 'file-card-title' }, f.file_name), h('p', { class: 'path mono', tip: f.path }, f.path)),
      h('span', { class: 'badge badge-ok' }, t('load.card.ready'))),
    h('dl', { class: 'facts' },
      fact(h('span', { class: 'term', tip: t('tip.schema') }, t('term.schema')), f.schema),
      fact(t('load.card.size'), bytes(f.size_bytes)),
      fact(t('load.card.elements'), nf(f.element_count)),
      fact(t('load.card.units'), `${t(`units.${f.units.system}`)} · ${cap(f.units.length_unit || '')}`),
      fact(t('load.card.sha'), sha)),
    h('div', { class: 'chip-rows' },
      h('div', { class: 'chip-row' }, h('span', { class: 'chip-label' }, t('load.card.disciplines')),
        f.disciplines.length ? chipList(f.disciplines.map((d) => tOr(`discipline.${d}`, d))) : h('span', { class: 'muted' }, t('common.none'))),
      h('div', { class: 'chip-row' }, h('span', { class: 'chip-label' }, t('load.card.storeys')),
        f.storeys.length ? chipList(f.storeys) : h('span', { class: 'muted' }, t('common.none'))),
      h('div', { class: 'chip-row' }, h('span', { class: 'chip-label' }, t('load.card.types')),
        types.length ? chipList(types.slice(0, 8).map(([k, v]) => `${k} · ${nf(v)}`), 'mono-chips') : h('span', { class: 'muted' }, t('common.none')))),
    f.reader_warnings && f.reader_warnings.length
      ? h('div', { class: 'notice notice-warning' }, h('div', { class: 'notice-body' },
          h('strong', null, tn('load.card.warnings', f.reader_warnings.length)),
          h('ul', null, f.reader_warnings.map((w) => h('li', null, w)))))
      : null));
}

export function renderRunBar() {
  const btn = $('#btn-run');
  const title = $('#run-status-title');
  const sub = $('#run-status-sub');
  const prog = $('#run-progress');
  const block = blockingProblem();
  prog.hidden = !state.running;
  $('#run-stage').textContent = state.runStage;
  if (state.running) {
    title.textContent = t('run.running');
    sub.textContent = '';
    btn.disabled = true;
    btn.textContent = t('run.running_btn');
    return;
  }
  btn.textContent = t('load.run');
  if (!state.file) {
    title.textContent = t('run.need_file');
    sub.textContent = t('run.need_file_sub');
    btn.disabled = true;
  } else if (block) {
    title.textContent = t('run.fix_settings');
    sub.textContent = block;
    btn.disabled = true;
  } else {
    const cfg = state.configRaw != null ? t('run.cfg_file') : isDirty(state.settings) ? t('run.cfg_edited') : t('run.cfg_default');
    title.textContent = t('run.ready', { name: state.file.file_name });
    sub.textContent = t('run.ready_sub', { count: nf(state.file.element_count), ids: state.idsPath ? t('run.with_ids') : t('run.no_ids'), cfg });
    btn.disabled = false;
  }
}

export function renderLoad() {
  const dz = $('#dropzone');
  dz.classList.toggle('is-loaded', Boolean(state.file));
  $('#view-load').classList.toggle('has-file', Boolean(state.file || state.inspecting));
  $('#dz-title').textContent = state.file ? t('load.drop.title_loaded') : t('load.drop.title');
  $('#dz-sub').textContent = state.file ? t('load.drop.sub_loaded') : t('load.drop.sub');
  $('#btn-browse-ifc').textContent = state.file ? t('load.browse.ifc_replace') : t('load.browse.ifc');
  $('#btn-browse-ifc').disabled = state.inspecting || state.running;
  $('#demo-cta').hidden = !demoExample();
  $('#btn-demo').disabled = state.inspecting || state.running;
  renderAttachments();
  renderErrorCard();
  renderFileCard();
  renderSampleList();
  updateSettingsBadge();
  renderRunBar();
}

export function initLoad() {
  $('#btn-browse-ifc').addEventListener('click', async () => { const p = await api.pickFile('ifc'); if (p) loadIfc(p); });
  $('#btn-browse-ids').addEventListener('click', async () => { const p = await api.pickFile('ids'); if (p) attachIds(p); });
  $('#btn-browse-yaml').addEventListener('click', async () => { const p = await api.pickFile('yaml'); if (p) attachConfig(p); });
  $('#btn-clear-ids').addEventListener('click', () => { state.idsPath = null; renderLoad(); });
  $('#btn-clear-yaml').addEventListener('click', clearConfigFile);
  $('#btn-demo').addEventListener('click', () => { const ex = demoExample(); if (ex) useExample(ex); });
  $('#btn-run').addEventListener('click', () => actions.run());
}
