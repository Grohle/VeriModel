// VeriModel frontend entry point.
//
// Why vanilla HTML + CSS + ES modules (no framework, no bundler, no npm):
//  * No build step to break: the folder is the product. Tauri serves these
//    files as they are (frontendDist = ../frontend), and a developer can open
//    them over any static server.
//  * Nothing is fetched from a CDN or font host, so the app is fully offline
//    and the UI cannot silently depend on a third party.
//  * Auditable: a quality gate that BIM coordinators rely on should have a UI
//    whose complete source can be read in an afternoon, with no transitive
//    dependency tree to review.
import { $, $$, initTooltips, toast, hideTip } from './js/dom.js';
import { setLanguage, applyI18n, t } from './js/i18n.js';
import { state, actions, store } from './js/state.js';
import * as api from './js/api.js';
import { defaultSettings, effectiveYaml, loadDefaultYaml } from './js/settings.js';
import {
  initLoad, renderLoad, renderRunBar, renderSettingsPanel, updateSettingsBadge,
  handleDropped, clearConfigFile,
} from './js/view-load.js';
import { renderReview, refreshHead, setRuleFilter, closeVerdictPopover } from './js/view-review.js';
import { renderExport } from './js/view-export.js';
import { initDetail, openDetail, closeDetail, isDetailOpen, renderDetail } from './js/detail.js';

const VIEWS = { 1: '#view-load', 2: '#view-review', 3: '#view-export' };
const stepAvailable = (n) => n === 1 || Boolean(state.result);

// ------------------------------------------------------------ preferences

function applyTheme() {
  document.documentElement.dataset.theme = state.theme;
  $$('[data-theme-set]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.themeSet === state.theme)));
}
function applyToggleState() {
  applyTheme();
  $$('[data-lang-set]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.langSet === state.lang)));
  $$('[data-units-set]').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.unitsSet === state.units)));
}

async function setLang(lang) {
  state.lang = lang;
  store.set('vm.lang', lang);
  await setLanguage(lang);
  applyI18n();
  applyToggleState();
  renderSettingsPanel();
  renderAll();
}

// ---------------------------------------------------------------- routing

function renderStepper() {
  $$('#stepper .step').forEach((b) => {
    const n = Number(b.dataset.step);
    const current = n === state.step;
    b.disabled = !stepAvailable(n);
    b.classList.toggle('is-current', current);
    b.classList.toggle('is-done', !current && (n === 1 ? Boolean(state.result) : n < state.step));
    if (current) b.setAttribute('aria-current', 'step'); else b.removeAttribute('aria-current');
  });
}

function renderAll() {
  renderStepper();
  Object.entries(VIEWS).forEach(([n, sel]) => { $(sel).hidden = Number(n) !== state.step; });
  document.body.dataset.step = String(state.step);
  if (state.step === 1) renderLoad();
  if (state.step === 2) renderReview();
  if (state.step === 3) renderExport();
}

async function goto(n, { focus = true } = {}) {
  if (!stepAvailable(n)) return;
  if (n !== 2) closeDetail();
  state.step = n;
  if (n === 3) {
    try { state.runs = await api.listRuns(null); state.runsError = null; }
    catch (e) { state.runs = []; state.runsError = e; }
  }
  renderAll();
  window.scrollTo({ top: 0 });
  if (focus) $(`${VIEWS[n]} h1`).focus({ preventScroll: true });
}

// ------------------------------------------------------------------ run

function setBusy(on) { $('#busy').hidden = !on; }

async function run() {
  if (state.running || !state.file) return;
  state.running = true;
  state.loadError = null;
  state.runStage = t('run.stage.read');
  setBusy(true);
  renderRunBar();
  refreshHead();
  const timer = setTimeout(() => { state.runStage = t('run.stage.check'); renderRunBar(); refreshHead(); }, 1100);
  try {
    const first = !state.result;
    const res = await api.runCheck({
      ifc_path: state.file.path,
      ids_path: state.idsPath,
      config_yaml: effectiveYaml(),
      out_dir: null,
    });
    state.result = res;
    state.review.filters = { sev: { ERROR: true, WARNING: true, INFO: true }, rule: '', storey: '', type: '', q: '' };
    state.review.page = { checks: 1, failures: 1, quantities: 1 };
    if (first) state.review.tab = 'checks';
    closeDetail();
    state.running = false;
    await goto(2);
    if (!first) toast(t('run.done_again'), 'success');
  } catch (e) {
    state.running = false;
    state.loadError = e;
    await goto(1);
  } finally {
    clearTimeout(timer);
    state.running = false;
    state.runStage = '';
    setBusy(false);
    if (state.step === 1) renderLoad(); else refreshHead();
  }
}

function newCheck() {
  // Fresh model, same configuration: settings, project.yaml and IDS stay.
  state.file = null;
  state.loadError = null;
  goto(1);
}

// ------------------------------------------------------------------ boot

async function boot() {
  initTooltips();
  state.settings = defaultSettings();
  await setLanguage(state.lang);

  Object.assign(actions, {
    goto, run, newCheck, toast, openDetail, clearConfigFile,
    filterByRule: setRuleFilter,
    settingsChanged: () => { renderRunBar(); updateSettingsBadge(); },
  });

  // toggles
  $$('[data-theme-set]').forEach((b) => b.addEventListener('click', () => {
    state.theme = b.dataset.themeSet; store.set('vm.theme', state.theme); applyTheme();
  }));
  $$('[data-lang-set]').forEach((b) => b.addEventListener('click', () => setLang(b.dataset.langSet)));
  $$('[data-units-set]').forEach((b) => b.addEventListener('click', () => {
    state.units = b.dataset.unitsSet; store.set('vm.units', state.units); applyToggleState(); renderAll();
    if (isDetailOpen()) renderDetail();
  }));
  $$('#stepper .step').forEach((b) => b.addEventListener('click', () => goto(Number(b.dataset.step))));

  applyI18n();
  applyToggleState();
  initLoad();
  initDetail();
  renderSettingsPanel();

  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return;
    hideTip();
    if (closeVerdictPopover()) { const b = $('#btn-verdict-info'); if (b) b.focus(); return; }
    if (closeDetail()) e.preventDefault();
  });

  const [examples] = await Promise.all([api.listExamples().catch(() => []), loadDefaultYaml()]);
  state.examples = examples;
  renderSettingsPanel();
  renderAll();

  api.onDragDrop({
    enter: () => $('#dropzone').classList.add('is-dragging'),
    leave: () => $('#dropzone').classList.remove('is-dragging'),
    drop: async (paths) => { if (state.step !== 1) await goto(1, { focus: false }); handleDropped(paths); },
  });
}

boot().catch((e) => {
  console.error(e);
  document.body.insertAdjacentText('afterbegin', t('app.start_failed'));
});
