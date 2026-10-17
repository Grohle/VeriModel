// Project settings: the form model, its YAML projection and the form itself.
// Defaults reproduce the engine defaults, so an untouched form sends no
// config at all (config_yaml = null) and the engine runs with zero config.
import { h, clear, setTip } from './dom.js';
import { t } from './i18n.js';
import { emitYaml } from './yaml.js';
import { state, actions } from './state.js';
import * as api from './api.js';
import { basename } from './format.js';

export const GROUP_KEYS = ['type', 'storey', 'classification', 'material', 'predefined_type', 'type_name'];
export const CLASSIFICATION_SYSTEMS = ['none', 'Uniclass', 'OmniClass', 'CCI', 'DIN276'];

export function defaultSettings() {
  return {
    rulePack: 'generic', rulePackPath: '',
    farFromOrigin: 10000, minVolume: 0.000001,
    classificationSystem: 'none', classificationRequired: false,
    groupBy: ['type', 'storey'], currency: 'EUR', ratesFile: '',
    reportLanguage: null, // null = follow the interface language
  };
}

const reportLang = (s) => s.reportLanguage || state.lang;

export function settingsToTree(s) {
  const qto = { group_by: GROUP_KEYS.filter((k) => s.groupBy.includes(k)), currency: s.currency.toUpperCase() };
  if (s.ratesFile.trim()) qto.unit_rates_file = s.ratesFile.trim();
  return {
    project: { classification_system: s.classificationSystem },
    requirements: { rule_pack: s.rulePack === 'custom' ? s.rulePackPath.trim() : 'generic' },
    tolerances: { far_from_origin_m: s.farFromOrigin, geometry: { min_volume_m3: s.minVolume } },
    classification: { required: s.classificationRequired },
    qto,
    output: { language: reportLang(s) },
  };
}

export function settingsToYaml(s) {
  return `${t('settings.yaml.header')}\n${emitYaml(settingsToTree(s))}\n`;
}

export function isDirty(s) {
  const base = { ...defaultSettings(), reportLanguage: 'en' };
  return JSON.stringify(settingsToTree(s)) !== JSON.stringify(settingsToTree(base));
}

// The YAML sent to run_check: raw file > edited form > null (engine defaults).
export function effectiveYaml() {
  if (state.configRaw != null) return state.configRaw;
  return isDirty(state.settings) ? settingsToYaml(state.settings) : null;
}

export function localProblems(s) {
  const p = [];
  if (s.rulePack === 'custom' && !s.rulePackPath.trim()) p.push('settings.err.rule_pack_path');
  if (!(s.farFromOrigin > 0)) p.push('settings.err.far');
  if (!(s.minVolume >= 0)) p.push('settings.err.min_volume');
  if (!s.groupBy.length) p.push('settings.err.group_by');
  if (!/^[A-Za-z]{3}$/.test(s.currency.trim())) p.push('settings.err.currency');
  return p;
}

// ------------------------------------------------------------------- form

let validateTimer = null;

export function renderSettings(root) {
  clear(root);
  const s = state.settings;
  const locked = state.configRaw != null;

  const onChange = () => { refresh(); actions.settingsChanged(); };

  const field = (id, labelKey, hintKey, control, tipKey) =>
    h('div', { class: 'field' },
      h('label', { for: id }, h('span', tipKey ? { class: 'term', tip: t(tipKey) } : null, t(labelKey))),
      control,
      hintKey ? h('p', { class: 'hint' }, t(hintKey)) : null);

  const numInput = (id, key, step, min) => h('input', {
    id, type: 'number', step, min, value: String(s[key]), inputmode: 'decimal',
    oninput: (e) => { s[key] = e.target.value === '' ? NaN : Number(e.target.value); onChange(); },
  });

  // --- rule pack
  const packPath = h('div', { class: 'path-row', hidden: s.rulePack !== 'custom' },
    h('input', { id: 'set-pack-path', type: 'text', value: s.rulePackPath, placeholder: t('settings.pack.placeholder'),
      oninput: (e) => { s.rulePackPath = e.target.value; onChange(); } }),
    h('button', { type: 'button', class: 'btn btn-secondary', onclick: async () => {
      const p = await api.pickFile('yaml'); if (p) { s.rulePackPath = p; renderSettings(root); onChange(); }
    } }, t('common.browse')));
  const packSelect = h('select', { id: 'set-pack', onchange: (e) => {
    s.rulePack = e.target.value; packPath.hidden = s.rulePack !== 'custom'; onChange();
  } },
    h('option', { value: 'generic', selected: s.rulePack === 'generic' }, t('settings.pack.generic')),
    h('option', { value: 'custom', selected: s.rulePack === 'custom' }, t('settings.pack.custom')));

  // --- classification
  const clsSelect = h('select', { id: 'set-cls', onchange: (e) => { s.classificationSystem = e.target.value; onChange(); } },
    CLASSIFICATION_SYSTEMS.map((c) => h('option', { value: c, selected: s.classificationSystem === c }, c === 'none' ? t('settings.cls.none') : c)));
  const clsRequired = h('label', { class: 'switch' },
    h('input', { type: 'checkbox', id: 'set-cls-required', checked: s.classificationRequired, onchange: (e) => { s.classificationRequired = e.target.checked; onChange(); } }),
    h('span', { class: 'switch-track', 'aria-hidden': 'true' }),
    h('span', null, t('settings.cls.required')));

  // --- QTO
  const groupChecks = h('div', { class: 'check-grid', role: 'group', 'aria-labelledby': 'set-group-label' },
    GROUP_KEYS.map((k) => h('label', { class: 'check' },
      h('input', { type: 'checkbox', value: k, checked: s.groupBy.includes(k), onchange: (e) => {
        s.groupBy = e.target.checked ? [...s.groupBy, k] : s.groupBy.filter((x) => x !== k); onChange();
      } }),
      h('span', null, t(`group.${k}`)))));
  const rates = h('div', { class: 'path-row' },
    h('input', { id: 'set-rates', type: 'text', value: s.ratesFile, placeholder: t('settings.rates.placeholder'),
      oninput: (e) => { s.ratesFile = e.target.value; onChange(); } }),
    h('button', { type: 'button', class: 'btn btn-secondary', onclick: async () => {
      const p = await api.pickFile('csv'); if (p) { s.ratesFile = p; renderSettings(root); onChange(); }
    } }, t('common.browse')));

  const langSelect = h('select', { id: 'set-lang', onchange: (e) => { s.reportLanguage = e.target.value; onChange(); } },
    h('option', { value: 'en', selected: reportLang(s) === 'en' }, t('settings.lang.en')),
    h('option', { value: 'es', selected: reportLang(s) === 'es' }, t('settings.lang.es')));

  const form = h('fieldset', { class: 'settings-form', disabled: locked },
    h('legend', { class: 'sr-only' }, t('settings.title')),
    h('div', { class: 'settings-col' },
      h('h3', null, t('settings.sec.rules')),
      field('set-pack', 'settings.pack.label', 'settings.pack.hint', packSelect),
      packPath,
      field('set-far', 'settings.far.label', 'settings.far.hint', numInput('set-far', 'farFromOrigin', 'any', '0')),
      field('set-minvol', 'settings.minvol.label', 'settings.minvol.hint', numInput('set-minvol', 'minVolume', 'any', '0')),
      h('h3', null, t('settings.sec.classification')),
      field('set-cls', 'settings.cls.label', 'settings.cls.hint', clsSelect),
      h('div', { class: 'field' }, clsRequired, h('p', { class: 'hint' }, t('settings.cls.required_hint')))),
    h('div', { class: 'settings-col' },
      h('h3', null, h('span', { class: 'term', tip: t('tip.qto') }, t('settings.sec.qto'))),
      h('div', { class: 'field' },
        h('span', { class: 'label', id: 'set-group-label' }, t('settings.group.label')),
        groupChecks,
        h('p', { class: 'hint' }, t('settings.group.hint'))),
      field('set-currency', 'settings.currency.label', null, h('input', { id: 'set-currency', type: 'text', maxlength: '3', value: s.currency, class: 'short',
        oninput: (e) => { s.currency = e.target.value; onChange(); } })),
      field('set-rates', 'settings.rates.label', 'settings.rates.hint', rates),
      h('h3', null, t('settings.sec.report')),
      field('set-lang', 'settings.lang.label', 'settings.lang.hint', langSelect)));

  const banner = locked
    ? h('div', { class: 'notice notice-info' },
        h('div', null, h('strong', null, t('settings.locked.title', { name: basename(state.configPath || 'project.yaml') })), h('p', null, t('settings.locked.body'))),
        h('button', { type: 'button', class: 'btn btn-secondary', onclick: () => actions.clearConfigFile() }, t('settings.locked.use_form')))
    : null;

  const preview = h('pre', { id: 'yaml-preview', class: 'yaml', tabindex: '0', 'aria-label': t('settings.preview.label') });
  const problems = h('ul', { id: 'settings-problems', class: 'problems', role: 'status' });

  const side = h('div', { class: 'settings-side' },
    h('div', { class: 'yaml-head' },
      h('h3', null, t('settings.preview.title')),
      h('span', { class: 'muted small', id: 'yaml-note' })),
    preview,
    problems,
    h('div', { class: 'settings-actions' },
      h('button', { type: 'button', class: 'btn btn-secondary', id: 'btn-download-yaml', onclick: async () => {
        try {
          const saved = await api.saveTextFile(currentYamlText(), 'project.yaml');
          if (saved) actions.toast(t('settings.saved', { name: basename(saved) }), 'success');
        } catch (e) { actions.toast(e.message, 'error'); }
      } }, t('settings.download')),
      h('button', { type: 'button', class: 'btn btn-ghost', onclick: () => { state.settings = defaultSettings(); renderSettings(root); onChange(); } }, t('settings.reset'))));

  if (banner) root.appendChild(banner);
  root.appendChild(h('div', { class: 'settings-grid' }, form, side));
  refresh();

  function currentYamlText() {
    if (state.configRaw != null) return state.configRaw;
    return isDirty(state.settings) ? settingsToYaml(state.settings) : defaultYamlCache;
  }

  function refresh() {
    const raw = state.configRaw != null;
    const dirty = isDirty(state.settings);
    preview.textContent = currentYamlText();
    root.querySelector('#yaml-note').textContent = raw ? t('settings.preview.raw') : dirty ? t('settings.preview.edited') : t('settings.preview.defaults');
    const probs = raw ? [] : localProblems(state.settings);
    clear(problems);
    probs.forEach((k) => problems.appendChild(h('li', null, t(k))));
    if (state.configError) problems.appendChild(h('li', null, state.configError));
    clearTimeout(validateTimer);
    validateTimer = setTimeout(async () => {
      const yaml = effectiveYaml();
      if (yaml == null || localProblems(state.settings).length && !raw) { state.configError = null; actions.settingsChanged(); return; }
      try {
        const r = await api.validateConfigYaml(yaml);
        state.configError = r.ok ? null : (r.error || t('settings.err.generic'));
      } catch (e) { state.configError = e.message; }
      refreshProblemsOnly();
      actions.settingsChanged();
    }, 350);
  }

  function refreshProblemsOnly() {
    const raw = state.configRaw != null;
    clear(problems);
    (raw ? [] : localProblems(state.settings)).forEach((k) => problems.appendChild(h('li', null, t(k))));
    if (state.configError) problems.appendChild(h('li', null, state.configError));
  }
}

// Default YAML text shown while nothing was edited (comes from the engine).
let defaultYamlCache = '';
export async function loadDefaultYaml() {
  try { defaultYamlCache = await api.defaultConfigYaml(); } catch { defaultYamlCache = settingsToYaml(defaultSettings()); }
}

export function blockingProblem() {
  if (state.configRaw != null) return state.configError ? state.configError : null;
  const p = localProblems(state.settings);
  if (p.length) return t(p[0]);
  return state.configError;
}
