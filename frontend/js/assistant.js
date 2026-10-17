// Assistant side panel: a local, optional, READ-ONLY copilot.
// It helps you search the report, understand a finding and write settings.
// It never decides pass/fail and never produces a score or a quantity: every
// answer is a suggestion the user applies with a button.
import { h, clear, $, toast } from './dom.js';
import { t, tn } from './i18n.js';
import { state, actions } from './state.js';
import * as api from './api.js';
import { nf, basename } from './format.js';
import { applyFilterSpec } from './view-review.js';
import { effectiveYaml } from './settings.js';

const A = state.assistant;
const SEVERITIES = ['ERROR', 'WARNING', 'INFO'];
let opener = null;
let removeTimer = null;
let nextId = 1;

export const isAssistantOpen = () => A.open;

// ------------------------------------------------------------------ helpers

const gb = (n) => `${nf(n / 1e9, 2)} GB`;
const hasFinding = () => Boolean(state.detail && state.detail.finding);
const reportOf = () => (state.result ? state.result.report : null);

// Allowed values for a filter answer, taken from the current report.
function reportContext() {
  const r = reportOf();
  if (!r) return { rule_ids: [], storeys: [], ifc_types: [], severities: SEVERITIES };
  const uniq = (list) => [...new Set(list.filter(Boolean))];
  return {
    rule_ids: r.rules.map((x) => x.rule_id),
    storeys: uniq([...r.model.storeys, ...r.findings.map((x) => x.storey)]),
    ifc_types: uniq(r.findings.map((x) => x.ifc_type)).sort(),
    severities: SEVERITIES,
  };
}

function buildTask(mode, text) {
  if (mode === 'filter') return { kind: 'filter', question: text, context: reportContext() };
  if (mode === 'explain') {
    const finding = state.detail.finding;
    const rule = reportOf().rules.find((r) => r.rule_id === finding.rule_id) || null;
    return { kind: 'explain', finding, rule };
  }
  if (mode === 'config') {
    return { kind: 'config_help', question: text, current_yaml: effectiveYaml() };
  }
  return { kind: 'ask', question: text };
}

function prefillFor(mode) {
  const r = reportOf();
  if (mode === 'filter') {
    const storey = r && (r.model.storeys[0] || (r.findings.find((f) => f.storey) || {}).storey);
    return t('assistant.prefill.find_errors', { storey: storey || '…' });
  }
  if (mode === 'explain') return t('assistant.prefill.explain');
  if (mode === 'config') return t('assistant.prefill.config');
  return '';
}

// ------------------------------------------------------------------- status

export async function refreshStatus() {
  try { A.status = await api.assistantStatus(); A.statusError = null; }
  catch (e) { A.statusError = e; }
  renderStatus();
  renderComposer();
}

function renderStatus() {
  const host = $('#as-status');
  clear(host);
  const s = A.status;
  if (!s) {
    host.appendChild(h('p', { class: 'muted small' }, A.statusError ? A.statusError.message || t('assistant.status.error') : '…'));
    return;
  }
  const noBackend = s.backend === 'none' || !s.backend;
  const facts = h('dl', { class: 'as-facts' },
    h('div', null, h('dt', null, t('assistant.status.size')), h('dd', null, gb(s.gguf_size_bytes))),
    h('div', null, h('dt', null, t('assistant.status.license')), h('dd', null, s.license)),
    h('div', null, h('dt', null, t('assistant.status.ram')), h('dd', null, t('assistant.status.ram_value', { mb: nf(s.approx_ram_mb) }))),
    h('div', null, h('dt', null, t('assistant.status.backend')), h('dd', null, noBackend ? t('assistant.status.backend_none') : s.backend)));
  const dl = A.download;
  const stateLabel = s.installed
    ? h('span', { class: 'as-state is-ready' }, h('span', { class: 'as-dot', 'aria-hidden': 'true' }), t('assistant.status.ready'))
    : h('span', { class: 'as-state' }, h('span', { class: 'as-dot', 'aria-hidden': 'true' }), t('assistant.status.missing'));
  host.appendChild(h('div', { class: 'as-card-head' }, h('h3', null, t('assistant.status.title')), stateLabel));
  host.appendChild(h('p', { class: 'as-model' }, s.name));
  host.appendChild(facts);

  if (s.installed) {
    host.appendChild(h('p', { class: 'muted small' }, s.loaded ? t('assistant.status.loaded_sub') : t('assistant.status.ready_sub')));
    host.appendChild(h('div', { class: 'as-actions' },
      h('button', { type: 'button', id: 'as-remove', class: `btn btn-ghost btn-sm`, disabled: A.busy || Boolean(dl), onclick: onRemove },
        A.confirmRemove ? t('assistant.remove_confirm') : t('assistant.remove'))));
    return;
  }

  host.appendChild(h('p', { class: 'as-explain' }, t('assistant.status.explain', { size: gb(s.gguf_size_bytes) })));
  if (dl) {
    const pct = dl.total ? Math.min(100, Math.floor((dl.done * 100) / dl.total)) : 0;
    host.appendChild(h('div', { class: 'as-progress' },
      h('div', { class: 'progress-track', role: 'progressbar', 'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': String(pct), 'aria-label': t('assistant.download_progress'), id: 'as-progressbar' },
        h('div', { class: 'progress-bar', style: `width:${pct}%` })),
      h('p', { class: 'small muted', id: 'as-progress-text' }, t('assistant.downloading', { pct }))));
  } else {
    host.appendChild(h('div', { class: 'as-actions' },
      h('button', { type: 'button', id: 'as-download', class: 'btn btn-primary', disabled: noBackend, onclick: onDownload }, t('assistant.download'))));
    if (noBackend) host.appendChild(h('p', { class: 'small muted' }, t('assistant.status.backend_none')));
    if (A.statusError) host.appendChild(h('p', { class: 'small', style: 'color:var(--error)' }, [A.statusError.message, ' ', A.statusError.hint || ''].join('')));
  }
}

function updateProgress() {
  const dl = A.download;
  const bar = $('#as-progressbar');
  if (!dl || !bar) return;
  const pct = dl.total ? Math.min(100, Math.floor((dl.done * 100) / dl.total)) : 0;
  bar.setAttribute('aria-valuenow', String(pct));
  bar.firstChild.style.width = `${pct}%`;
  $('#as-progress-text').textContent = t('assistant.downloading', { pct });
}

async function onDownload() {
  A.download = { done: 0, total: (A.status && A.status.gguf_size_bytes) || 0 };
  A.statusError = null;
  renderStatus();
  try {
    A.status = await api.assistantDownloadModel((p) => { A.download = { done: p.done, total: p.total || A.download.total }; updateProgress(); });
    toast(t('assistant.downloaded'), 'success');
  } catch (e) {
    A.statusError = e;
  } finally {
    A.download = null;
    renderStatus();
    renderComposer();
  }
}

async function onRemove() {
  if (!A.confirmRemove) {
    A.confirmRemove = true;
    renderStatus();
    clearTimeout(removeTimer);
    removeTimer = setTimeout(() => { A.confirmRemove = false; renderStatus(); }, 4000);
    $('#as-remove').focus();
    return;
  }
  clearTimeout(removeTimer);
  A.confirmRemove = false;
  try { A.status = await api.assistantRemoveModel(); toast(t('assistant.removed'), 'info'); }
  catch (e) { A.statusError = e; }
  renderStatus();
  renderComposer();
}

// -------------------------------------------------------------------- chips

function renderChips() {
  const host = $('#as-chips');
  clear(host);
  const r = reportOf();
  const chip = (mode, label, onclick, { disabled = false, tip = null } = {}) =>
    h('button', { type: 'button', class: 'chip-btn', 'aria-pressed': String(A.mode === mode), disabled, title: tip, onclick }, label);
  host.append(
    chip('filter', t('assistant.chip.find_errors'), () => pickMode('filter')),
    chip('explain', t('assistant.chip.explain'), () => pickMode('explain'), { disabled: !hasFinding(), tip: hasFinding() ? null : t('assistant.chip.explain_disabled') }),
    chip('config', t('assistant.chip.config'), () => pickMode('config')),
    h('button', { type: 'button', class: 'chip-btn', onclick: () => {
      const failing = r && r.rules.find((x) => x.failures > 0);
      pickMode('ask', t('assistant.prefill.rule', { rule: failing ? failing.rule_id : (r && r.rules[0] ? r.rules[0].rule_id : 'CL-001') }));
    } }, t('assistant.chip.rule')));
}

function pickMode(mode, text) {
  // Clicking the active chip again goes back to a general question.
  if (A.mode === mode && mode !== 'ask' && !text) { A.mode = 'ask'; renderChips(); renderComposer(); return; }
  A.mode = mode;
  const input = $('#as-input');
  input.value = text != null ? text : prefillFor(mode);
  renderChips();
  renderComposer();
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
}

// --------------------------------------------------------------- conversation

function filterChips(filter) {
  const items = [];
  if (filter.severity && filter.severity.length) items.push([t('assistant.filter.severity'), filter.severity.map((x) => t(`sev.${x}`)).join(', ')]);
  if (filter.rule_id) items.push([t('assistant.filter.rule'), filter.rule_id]);
  if (filter.storey) items.push([t('assistant.filter.storey'), filter.storey]);
  if (filter.ifc_type) items.push([t('assistant.filter.type'), filter.ifc_type]);
  if (filter.search) items.push([t('assistant.filter.search'), filter.search]);
  return items;
}

async function copyText(text, doneKey) {
  try { await navigator.clipboard.writeText(text); toast(t(doneKey), 'success'); }
  catch { toast(t('detail.copy_failed'), 'error'); }
}

function useYaml(msg) {
  state.configRaw = msg.answer.yaml;
  state.configPath = null;
  state.configError = null;
  msg.applied = true;
  actions.settingsRefresh();
  toast(t('assistant.config.applied'), 'success');
  renderConvo();
}

function applyFilter(msg) {
  if (!reportOf()) return;
  applyFilterSpec(msg.answer.filter);
  if (state.step !== 2) actions.goto(2, { focus: false });
  msg.applied = true;
  toast(t('assistant.filter.applied'), 'success');
  renderConvo();
}

function renderAnswer(msg) {
  const a = msg.answer;
  const kids = [];
  if (a.kind === 'filter') {
    const items = filterChips(a.filter);
    kids.push(items.length
      ? h('ul', { class: 'filter-chips' }, items.map(([k, v]) => h('li', { class: 'chip' }, h('b', null, `${k}:`), v)))
      : h('p', { class: 'muted' }, t('assistant.filter.none')));
    if (a.filter.explanation) kids.push(h('p', null, a.filter.explanation));
    if (a.filter.dropped && a.filter.dropped.length) kids.push(h('p', { class: 'note-dropped' }, t('assistant.filter.dropped', { values: a.filter.dropped.join(', ') })));
    kids.push(h('div', { class: 'answer-actions' },
      h('button', { type: 'button', class: 'btn btn-secondary btn-sm', disabled: !reportOf() || !items.length, onclick: () => applyFilter(msg) }, t('assistant.filter.apply')),
      msg.applied ? h('span', { class: 'done' }, t('assistant.filter.applied')) : (!reportOf() ? h('span', { class: 'small muted' }, t('assistant.filter.need_report')) : null)));
  } else if (a.kind === 'explanation') {
    kids.push(h('p', null, a.text));
  } else if (a.kind === 'config_snippet') {
    kids.push(h('div', { class: 'answer-actions' },
      a.valid ? h('span', { class: 'badge badge-ok' }, t('assistant.config.valid'))
        : h('span', { class: 'badge sev-error' }, t('assistant.config.invalid', { error: a.validation_error || '?' }))));
    if (a.explanation) kids.push(h('p', null, a.explanation));
    kids.push(h('pre', { class: 'yaml-block mono', tabindex: '0' }, a.yaml));
    kids.push(h('div', { class: 'answer-actions' },
      h('button', { type: 'button', class: 'btn btn-secondary btn-sm', disabled: !a.valid, title: t('assistant.config.use_hint'), onclick: () => useYaml(msg) }, t('assistant.config.use')),
      h('button', { type: 'button', class: 'btn btn-ghost btn-sm', onclick: () => copyText(a.yaml, 'assistant.config.copied') }, t('assistant.config.copy')),
      msg.applied ? h('span', { class: 'done' }, t('assistant.config.applied')) : null));
  } else {
    if (a.structured === false) kids.push(h('p', { class: 'note-warn' }, t('assistant.unstructured')));
    kids.push(h('p', null, a.text));
    if (a.sources && a.sources.length) kids.push(h('p', { class: 'sources' }, t('assistant.sources', { list: a.sources.join(', ') })));
  }
  return h('div', { class: 'answer' }, kids);
}

function renderMessage(m) {
  if (m.role === 'user') return h('div', { class: 'msg msg-user' }, h('span', { class: 'msg-who' }, t('assistant.you')), h('div', { class: 'bubble' }, m.text));
  if (m.role === 'note') return h('div', { class: 'msg msg-assistant' }, h('div', { class: 'answer' }, h('p', null, m.text)));
  if (m.role === 'error') {
    return h('div', { class: 'msg msg-error' }, h('div', { class: 'answer' },
      h('strong', null, t('assistant.answer_error')), h('p', null, m.error.message), m.error.hint ? h('p', { class: 'muted' }, m.error.hint) : null));
  }
  return h('div', { class: 'msg msg-assistant' }, h('span', { class: 'msg-who' }, t('assistant.assistant')), renderAnswer(m));
}

function renderConvo({ scroll = true } = {}) {
  const host = $('#as-convo');
  clear(host);
  if (!A.messages.length && !A.busy) host.appendChild(h('p', { class: 'as-empty' }, t('assistant.empty')));
  A.messages.forEach((m) => host.appendChild(renderMessage(m)));
  if (A.busy) host.appendChild(h('div', { class: 'msg msg-assistant' }, h('div', { class: 'thinking', role: 'status' }, h('i'), h('i'), h('i'), h('span', null, t('assistant.thinking')))));
  if (scroll) { const sc = $('#assistant-scroll'); sc.scrollTop = sc.scrollHeight; }
}

// ------------------------------------------------------------------ composer

function renderComposer() {
  const installed = Boolean(A.status && A.status.installed);
  const input = $('#as-input');
  const locked = !installed || A.busy;
  input.disabled = locked;
  $('#as-send').disabled = locked;
  const hint = $('#as-hint');
  hint.hidden = installed;
  hint.textContent = installed ? '' : t('assistant.input.disabled');
  $('#as-mode').textContent = installed ? t(`assistant.mode.${A.mode}`) : '';
}

export async function send(modeOverride, textOverride) {
  const mode = modeOverride || A.mode;
  const input = $('#as-input');
  const text = (textOverride != null ? textOverride : input.value).trim();
  if (A.busy || !A.status || !A.status.installed) return;
  if (mode !== 'explain' && !text) return;
  const userText = mode === 'explain' ? (text || t('assistant.prefill.explain')) : text;
  if (mode === 'filter' && !reportOf()) {
    A.messages.push({ role: 'user', text: userText }, { role: 'note', text: t('assistant.need_report') });
    input.value = '';
    renderConvo();
    return;
  }
  if (mode === 'explain' && !hasFinding()) {
    A.messages.push({ role: 'user', text: userText }, { role: 'note', text: t('assistant.need_finding') });
    renderConvo();
    return;
  }
  const task = buildTask(mode, text);
  A.messages.push({ role: 'user', text: userText });
  input.value = '';
  A.busy = true;
  renderConvo();
  renderComposer();
  renderStatus();
  try {
    const answer = await api.assistantAsk(task);
    A.messages.push({ role: 'assistant', id: nextId++, answer, applied: false });
  } catch (e) {
    A.messages.push({ role: 'error', error: e });
  } finally {
    A.busy = false;
    renderConvo();
    renderComposer();
    await refreshStatus();
    if (A.open && !input.disabled) input.focus();
  }
}

// ------------------------------------------------------------ import + panel

async function onImport(directory) {
  const paths = await api.pickDocs({ directory });
  if (!paths) return;
  A.importing = true;
  A.importResult = null;
  renderImport();
  try { A.importResult = await api.assistantImportDocs(paths); }
  catch (e) { A.importResult = { error: e }; }
  A.importing = false;
  renderImport();
}

function renderImport() {
  const host = $('#as-import-result');
  clear(host);
  $('#as-import-files').disabled = A.importing;
  $('#as-import-folder').disabled = A.importing;
  if (A.importing) { host.appendChild(h('p', { class: 'muted' }, t('assistant.import.working'))); return; }
  const r = A.importResult;
  if (!r) return;
  if (r.error) { host.appendChild(h('p', { style: 'color:var(--error)' }, [r.error.message, ' ', r.error.hint || ''].join(''))); return; }
  const summary = t('assistant.import.summary', { files: tn('assistant.import.files', r.files_read.length), chunks: tn('assistant.import.chunks', r.chunks_added) });
  host.appendChild(h('p', null, r.skipped.length ? `${summary}; ${t('assistant.import.skipped', { list: r.skipped.map(basename).join(', ') })}` : summary));
  host.appendChild(h('div', { class: 'as-actions' },
    h('button', { type: 'button', class: 'btn btn-secondary btn-sm', onclick: async () => {
      try {
        let yaml;
        try { yaml = await api.assistantKnowledgeYaml(); } catch { yaml = r.knowledge_yaml; }
        const saved = await api.saveTextFile(yaml || r.knowledge_yaml, 'knowledge.yaml');
        if (saved) toast(t('assistant.import.saved', { name: basename(saved) }), 'success');
      } catch (e) { toast(e.message, 'error'); }
    } }, t('assistant.import.save'))));
}

export function renderAssistant() {
  renderStatus();
  renderChips();
  renderConvo({ scroll: false });
  renderComposer();
  renderImport();
  renderVersion();
}

export function refreshChips() {
  if (A.mode === 'explain' && !hasFinding()) { A.mode = 'ask'; renderComposer(); }
  renderChips();
}

function renderVersion() {
  const pill = $('#version-pill');
  if (!state.appVersion) { pill.hidden = true; return; }
  pill.hidden = false;
  pill.textContent = t('app.version_pill', { version: state.appVersion });
  pill.setAttribute('aria-label', t('app.version_aria', { version: state.appVersion }));
}

export function openAssistant({ focusInput = true } = {}) {
  if (A.open) return;
  opener = document.activeElement;
  A.open = true;
  $('#assistant').hidden = false;
  document.body.classList.add('assistant-open');
  $('#btn-assistant').setAttribute('aria-expanded', 'true');
  renderAssistant();
  refreshStatus();
  if (focusInput) {
    const input = $('#as-input');
    (input.disabled ? ($('#as-download') || $('#assistant-close')) : input).focus();
  }
}

export function closeAssistant() {
  if (!A.open) return false;
  A.open = false;
  $('#assistant').hidden = true;
  document.body.classList.remove('assistant-open');
  $('#btn-assistant').setAttribute('aria-expanded', 'false');
  const back = opener && document.contains(opener) && !opener.closest('#assistant') ? opener : $('#btn-assistant');
  if (back && back.focus) back.focus();
  return true;
}

// "Explain with assistant" in the finding detail.
export async function explainFinding() {
  openAssistant({ focusInput: false });
  A.mode = 'explain';
  renderChips();
  renderComposer();
  if (!A.status) await refreshStatus();
  await send('explain', '');
}

export async function initAssistant() {
  $('#btn-assistant').addEventListener('click', () => (A.open ? closeAssistant() : openAssistant()));
  $('#assistant-close').addEventListener('click', closeAssistant);
  $('#as-composer').addEventListener('submit', (e) => { e.preventDefault(); send(); });
  $('#as-input').addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); send(); }
  });
  $('#as-import-files').addEventListener('click', () => onImport(false));
  $('#as-import-folder').addEventListener('click', () => onImport(true));
  renderAssistant();
  try { state.appVersion = (await api.appInfo()).version; } catch { state.appVersion = null; }
  renderVersion();
  refreshStatus();
}
