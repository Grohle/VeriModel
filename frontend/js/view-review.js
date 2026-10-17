// Step 2: score + verdict header, five tabs, filters, paginated tables.
import { h, clear, $, $$, svg } from './dom.js';
import { t, tn } from './i18n.js';
import { state, actions } from './state.js';
import { nf, duration, basename } from './format.js';
import { fmtQty, fmtRate, unitLabel } from './units.js';
import { paginate, pager } from './table.js';

const SEVS = ['ERROR', 'WARNING', 'INFO'];
const TABS = ['checks', 'failures', 'quantities', 'ids', 'clash'];
const NONE_STOREY = '__none__';
const VERDICT_CLASS = { PASS: 'v-pass', PASS_WITH_WARNINGS: 'v-warn', FAIL: 'v-fail' };

const tOr = (key, fb) => { const v = t(key); return v === key ? fb : v; };
export const sevBadge = (sev) => h('span', { class: `badge sev-${sev.toLowerCase()}` }, t(`sev.${sev}`));
export const statusBadge = (st) => h('span', { class: `badge st-${st.toLowerCase()}` }, t(`status.${st}`));
const verdictKey = (v) => v || 'NONE';

// ---------------------------------------------------------------- derived

let derivedFor = null;
let derived = null;
function ctx() {
  const report = state.result.report;
  if (derivedFor !== report) {
    const findingIndex = new Map(report.findings.map((f, i) => [f, i]));
    derived = { report, findingIndex, rulesById: new Map(report.rules.map((r) => [r.rule_id, r])) };
    derivedFor = report;
  }
  return derived;
}
export const findingIndexOf = (f) => ctx().findingIndex.get(f);

const flt = () => state.review.filters;
const filtersActive = () => {
  const f = flt();
  return Boolean(f.rule || f.storey || f.type || f.q || SEVS.some((s) => !f.sev[s]));
};

function matchText(q, ...parts) {
  const needle = q.trim().toLowerCase();
  return !needle || parts.some((p) => p && String(p).toLowerCase().includes(needle));
}

function filteredFindings({ ignoreSev = false } = {}) {
  const f = flt();
  return ctx().report.findings.filter((x) =>
    (ignoreSev || f.sev[x.severity]) &&
    (!f.rule || x.rule_id === f.rule) &&
    (!f.storey || (f.storey === NONE_STOREY ? !x.storey : x.storey === f.storey)) &&
    (!f.type || x.ifc_type === f.type) &&
    matchText(f.q, x.global_id, x.name));
}

function filteredRules({ ignoreSev = false } = {}) {
  const f = flt();
  return ctx().report.rules.filter((r) => (ignoreSev || f.sev[r.severity]) && matchText(f.q, r.rule_id, r.description, r.category));
}

function qtoIndexes() {
  const gb = ctx().report.qto.group_by;
  return { type: gb.indexOf('type'), storey: gb.indexOf('storey') };
}

function filteredQtoRows() {
  const f = flt();
  const ix = qtoIndexes();
  return ctx().report.qto.rows.filter((r) =>
    (!f.type || ix.type < 0 || r.group[ix.type] === f.type) &&
    (!f.storey || ix.storey < 0 || (f.storey === NONE_STOREY ? !r.group[ix.storey] : r.group[ix.storey] === f.storey)) &&
    matchText(f.q, r.group.join(' ')));
}

// ------------------------------------------------------------------- head

function scoreRing(score, cls) {
  const C = 2 * Math.PI * 54;
  return h('div', { class: `score ${cls}` },
    svg('svg', { viewBox: '0 0 128 128', class: 'ring', role: 'img', 'aria-label': t('review.score_aria', { score }) },
      svg('circle', { class: 'ring-track', cx: 64, cy: 64, r: 54, fill: 'none', 'stroke-width': 10 }),
      svg('circle', { class: 'ring-value', cx: 64, cy: 64, r: 54, fill: 'none', 'stroke-width': 10, 'stroke-linecap': 'round',
        transform: 'rotate(-90 64 64)', 'stroke-dasharray': C.toFixed(2), 'stroke-dashoffset': (C * (1 - score / 100)).toFixed(2) })),
    h('div', { class: 'score-text' }, h('span', { class: 'score-num' }, String(score)), h('span', { class: 'score-of' }, t('review.score_of'))));
}

function closeVerdictPopover() {
  const pop = $('#verdict-pop');
  const btn = $('#btn-verdict-info');
  if (pop && !pop.hidden) { pop.hidden = true; if (btn) btn.setAttribute('aria-expanded', 'false'); return true; }
  return false;
}
export { closeVerdictPopover };

function renderHead() {
  const { report, ledger } = state.result;
  const s = report.summary;
  const vcls = VERDICT_CLASS[s.verdict] || 'v-none';
  const stat = (labelKey, value, cls = '') => h('div', { class: `stat ${cls}` }, h('dt', null, t(labelKey)), h('dd', null, value));

  const pop = h('div', { id: 'verdict-pop', class: 'popover', role: 'dialog', 'aria-label': t('review.verdict_how'), hidden: true },
    h('h3', null, t('review.verdict_how')),
    h('p', { class: 'verdict-rule' }, s.verdict_rule),
    h('button', { type: 'button', class: 'btn btn-secondary', onclick: () => { closeVerdictPopover(); $('#btn-verdict-info').focus(); } }, t('common.close')));
  const infoBtn = h('button', { type: 'button', id: 'btn-verdict-info', class: 'btn btn-link', 'aria-expanded': 'false', 'aria-controls': 'verdict-pop',
    onclick: (e) => { e.stopPropagation(); const open = pop.hidden; pop.hidden = !open; infoBtn.setAttribute('aria-expanded', String(open)); } }, t('review.verdict_how'));

  const busy = state.running;
  const head = $('#review-head');
  clear(head);
  head.append(
    scoreRing(s.score, vcls),
    h('div', { class: 'rh-body' },
      h('div', { class: 'rh-title' },
        h('h2', null, report.model.file_name),
        h('p', { class: 'muted' }, `${report.model.schema} · ${tn('review.elements_n', report.model.element_count)}${report.ids ? ` · ${t('review.with_ids')}` : ''}`)),
      h('div', { class: 'verdict-row' },
        h('span', { class: `badge verdict ${vcls}`, 'data-testid': 'verdict' }, t(`verdict.${verdictKey(s.verdict)}`)),
        h('div', { class: 'verdict-wrap' }, infoBtn, pop)),
      h('dl', { class: 'stats' },
        stat('review.stat.elements', nf(s.elements_checked)),
        stat('review.stat.errors', nf(s.errors), 'is-error'),
        stat('review.stat.warnings', nf(s.warnings), 'is-warning'),
        stat('review.stat.infos', nf(s.infos), 'is-info'),
        stat('review.stat.time', duration(ledger.duration_ms))),
      h('p', { class: 'muted small' }, t('review.rules_line', { passed: s.rules_passed, failed: s.rules_failed, skipped: s.rules_skipped, na: s.rules_not_applicable }))),
    h('div', { class: 'rh-actions' },
      busy ? h('p', { class: 'muted', role: 'status' }, state.runStage || t('run.running')) : null,
      h('button', { type: 'button', class: 'btn btn-primary', disabled: busy, onclick: () => actions.goto(3) }, t('review.to_export')),
      h('button', { type: 'button', class: 'btn btn-secondary', disabled: busy, onclick: () => actions.run() }, t('review.rerun')),
      h('button', { type: 'button', class: 'btn btn-secondary', disabled: busy, onclick: () => actions.goto(1) }, t('review.change_settings'))));
}

// ------------------------------------------------------------------- tabs

function tabCount(id) {
  const r = ctx().report;
  if (id === 'checks') return r.rules.length;
  if (id === 'failures') return r.findings.length;
  if (id === 'quantities') return r.qto.rows.length;
  if (id === 'ids') return r.ids ? r.ids.specifications.length : null;
  return null;
}

function renderTabs() {
  const list = $('#tablist');
  clear(list);
  const cur = state.review.tab;
  TABS.forEach((id, i) => {
    const n = tabCount(id);
    list.appendChild(h('button', {
      type: 'button', role: 'tab', id: `tab-${id}`, class: 'tab', tip: id === 'ids' ? t('tip.ids') : id === 'quantities' ? t('tip.qto') : null, 'aria-selected': String(id === cur), 'aria-controls': 'tabpanel', tabindex: id === cur ? '0' : '-1',
      onclick: () => setTab(id),
      onkeydown: (e) => {
        const move = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
        let next = null;
        if (move) next = TABS[(i + move + TABS.length) % TABS.length];
        else if (e.key === 'Home') next = TABS[0];
        else if (e.key === 'End') next = TABS[TABS.length - 1];
        if (next) { e.preventDefault(); setTab(next); $(`#tab-${next}`).focus(); }
      },
    }, t(`tab.${id}`), n != null ? h('span', { class: 'tab-count' }, nf(n)) : null));
  });
}

export function setTab(id) {
  state.review.tab = id;
  renderTabs();
  renderFilterbar();
  renderPanel();
}

// ---------------------------------------------------------------- filters

function select(id, labelKey, value, options, onChange) {
  return h('label', { class: 'filter-field', for: id }, h('span', null, t(labelKey)),
    h('select', { id, onchange: (e) => onChange(e.target.value) },
      options.map(([v, label]) => h('option', { value: v, selected: v === value }, label))));
}

function sevCount(sev) {
  const tab = state.review.tab;
  if (tab === 'checks') return filteredRules({ ignoreSev: true }).filter((r) => r.severity === sev).length;
  return filteredFindings({ ignoreSev: true }).filter((x) => x.severity === sev).length;
}

function updateFilterUi() {
  $$('#filterbar [data-sev]').forEach((b) => {
    const sev = b.dataset.sev;
    b.setAttribute('aria-pressed', String(flt().sev[sev]));
    b.querySelector('.count').textContent = nf(sevCount(sev));
  });
  const clearBtn = $('#btn-clear-filters');
  if (clearBtn) clearBtn.hidden = !filtersActive();
}

function onFilterChange() {
  state.review.page = { checks: 1, failures: 1, quantities: 1 };
  updateFilterUi();
  renderPanel();
}

export function setRuleFilter(ruleId) {
  const f = flt();
  f.rule = ruleId; f.storey = ''; f.type = ''; f.q = '';
  SEVS.forEach((s) => { f.sev[s] = true; });
  state.review.page.failures = 1;
  setTab('failures');
}

// Apply a filter proposed by the assistant (after the user pressed "Apply filter").
// `spec` is FilterSpec: severity[], rule_id, storey, ifc_type, search.
export function applyFilterSpec(spec) {
  const f = flt();
  f.rule = spec.rule_id || '';
  f.storey = spec.storey || '';
  f.type = spec.ifc_type || '';
  f.q = spec.search || '';
  const sevs = (spec.severity || []).map((x) => String(x).toUpperCase()).filter((x) => SEVS.includes(x));
  SEVS.forEach((s) => { f.sev[s] = sevs.length ? sevs.includes(s) : true; });
  state.review.page = { checks: 1, failures: 1, quantities: 1 };
  setTab('failures');
}

function renderFilterbar() {
  const bar = $('#filterbar');
  clear(bar);
  const tab = state.review.tab;
  const show = { checks: ['sev', 'q'], failures: ['sev', 'rule', 'storey', 'type', 'q'], quantities: ['storey', 'type', 'q'] }[tab];
  bar.hidden = !show;
  if (!show) return;
  const f = flt();
  const r = ctx().report;
  const all = (key) => ['', t(key)];

  if (show.includes('sev')) {
    bar.appendChild(h('div', { class: 'filter-group', role: 'group', 'aria-label': t('filter.severity') },
      SEVS.map((sev) => h('button', { type: 'button', class: `filter-chip sev-${sev.toLowerCase()}`, 'data-sev': sev, 'aria-pressed': String(f.sev[sev]),
        onclick: () => { f.sev[sev] = !f.sev[sev]; onFilterChange(); } },
      h('span', { class: 'dot', 'aria-hidden': 'true' }), t(`sev.${sev}`), h('span', { class: 'count' }, '0')))));
  }
  if (show.includes('rule')) {
    const opts = [all('filter.all_rules'), ...r.rules.filter((x) => x.failures > 0).map((x) => [x.rule_id, x.rule_id])];
    if (!opts.some(([v]) => v === f.rule)) f.rule = '';
    bar.appendChild(select('flt-rule', 'filter.rule', f.rule, opts, (v) => { f.rule = v; onFilterChange(); }));
  }
  if (show.includes('storey')) {
    let storeys;
    if (tab === 'quantities') {
      const ix = qtoIndexes().storey;
      storeys = ix < 0 ? [] : [...new Set(r.qto.rows.map((x) => x.group[ix]))];
    } else {
      storeys = [...new Set([...r.model.storeys, ...r.findings.map((x) => x.storey).filter(Boolean)])];
      if (r.findings.some((x) => !x.storey && x.global_id)) storeys.push(NONE_STOREY);
    }
    const opts = [all('filter.all_storeys'), ...storeys.map((s) => [s, s === NONE_STOREY ? t('filter.no_storey') : s])];
    if (!opts.some(([v]) => v === f.storey)) f.storey = '';
    bar.appendChild(select('flt-storey', 'filter.storey', f.storey, opts, (v) => { f.storey = v; onFilterChange(); }));
  }
  if (show.includes('type')) {
    let types;
    if (tab === 'quantities') { const ix = qtoIndexes().type; types = ix < 0 ? [] : [...new Set(r.qto.rows.map((x) => x.group[ix]))]; }
    else types = [...new Set(r.findings.map((x) => x.ifc_type).filter(Boolean))].sort();
    const opts = [all('filter.all_types'), ...types.map((x) => [x, x])];
    if (!opts.some(([v]) => v === f.type)) f.type = '';
    bar.appendChild(select('flt-type', 'filter.type', f.type, opts, (v) => { f.type = v; onFilterChange(); }));
  }
  if (show.includes('q')) {
    const ph = tab === 'failures' ? 'filter.search_failures' : tab === 'checks' ? 'filter.search_checks' : 'filter.search_qto';
    bar.appendChild(h('label', { class: 'filter-field filter-search', for: 'flt-q' }, h('span', null, t('filter.search')),
      h('input', { id: 'flt-q', type: 'search', value: f.q, placeholder: t(ph), autocomplete: 'off', oninput: (e) => { f.q = e.target.value; onFilterChange(); } })));
  }
  bar.appendChild(h('button', { type: 'button', id: 'btn-clear-filters', class: 'btn btn-ghost', hidden: !filtersActive(), onclick: () => {
    f.rule = ''; f.storey = ''; f.type = ''; f.q = ''; SEVS.forEach((s) => { f.sev[s] = true; }); renderFilterbar(); onFilterChange();
  } }, t('filter.clear')));
  updateFilterUi();
}

// ----------------------------------------------------------------- panels

function emptyState(titleKey, bodyKey, action) {
  return h('div', { class: 'empty' }, h('h3', null, t(titleKey)), bodyKey ? h('p', null, t(bodyKey)) : null, action || null);
}

function tableWrap(...kids) { return h('div', { class: 'table-wrap', tabindex: '-1' }, ...kids); }

function pageSlice(tab, items) {
  const info = paginate(items, state.review.page[tab], state.review.pageSize);
  state.review.page[tab] = info.page;
  return info;
}

function pagerEl(tab, info) {
  return pager(info, state.review.pageSize, {
    onPage: (p) => { state.review.page[tab] = p; renderPanel(); },
    onSize: (n) => { state.review.pageSize = n; state.review.page = { checks: 1, failures: 1, quantities: 1 }; renderPanel(); },
  });
}

function panelChecks() {
  const rules = filteredRules();
  if (!rules.length) return emptyState('empty.filters.title', 'empty.filters.body', clearButton());
  const info = pageSlice('checks', rules);
  const th = (k, cls = '') => h('th', { scope: 'col', class: cls }, t(k));
  return h('div', null,
    tableWrap(h('table', { class: 'data-table checks-table' },
      h('caption', { class: 'sr-only' }, t('tab.checks')),
      h('thead', null, h('tr', null, th('col.rule'), th('col.category'), th('col.description'), th('col.severity'), th('col.status'), th('col.checked', 'num'), th('col.failures', 'num'))),
      h('tbody', null, info.rows.map((r) => {
        const clickable = r.failures > 0;
        const go = () => actions.filterByRule(r.rule_id);
        return h('tr', { class: clickable ? 'is-clickable' : '', onclick: clickable ? go : null },
          h('td', null, clickable
            ? h('button', { type: 'button', class: 'linklike mono', 'aria-label': t('checks.show_failures', { rule: r.rule_id, n: r.failures }), onclick: (e) => { e.stopPropagation(); go(); } }, r.rule_id)
            : h('span', { class: 'mono' }, r.rule_id)),
          h('td', null, tOr(`category.${r.category}`, r.category)),
          h('td', { class: 'wrap' }, r.description,
            r.status === 'SKIPPED' && r.skipped_reason ? h('p', { class: 'reason' }, h('strong', null, t('checks.skipped_because')), ' ', r.skipped_reason) : null),
          h('td', null, sevBadge(r.severity)),
          h('td', null, statusBadge(r.status)),
          h('td', { class: 'num' }, nf(r.elements_checked)),
          h('td', { class: `num ${r.failures ? 'fail-count' : ''}` }, nf(r.failures)));
      })))),
    pagerEl('checks', info),
    h('p', { class: 'muted small tab-hint' }, t('checks.hint')));
}

function clearButton() {
  return h('button', { type: 'button', class: 'btn btn-secondary', onclick: () => {
    const f = flt(); f.rule = ''; f.storey = ''; f.type = ''; f.q = ''; SEVS.forEach((s) => { f.sev[s] = true; }); renderFilterbar(); onFilterChange();
  } }, t('filter.clear'));
}

function globalIdCell(f) {
  if (!f.global_id) return h('button', { type: 'button', class: 'linklike', onclick: (e) => { e.stopPropagation(); actions.openDetail(f); } }, t('failures.model_level'));
  return h('button', { type: 'button', class: 'linklike mono', 'aria-label': t('failures.open_element', { id: f.global_id }), onclick: (e) => { e.stopPropagation(); actions.openDetail(f); } }, f.global_id);
}

const gidHead = () => h('th', { scope: 'col' }, h('span', { class: 'term', tip: t('tip.globalid') }, t('col.globalid')));

function panelFailures() {
  const r = ctx().report;
  if (!r.findings.length) {
    return emptyState('empty.nofailures.title', 'empty.nofailures.body');
  }
  const list = filteredFindings();
  if (!list.length) return emptyState('empty.filters.title', 'empty.filters.body', clearButton());
  const info = pageSlice('failures', list);
  const th = (k, cls = '') => h('th', { scope: 'col', class: cls }, t(k));
  const sel = state.detail && state.detail.finding;
  return h('div', null,
    tableWrap(h('table', { class: 'data-table failures-table' },
      h('caption', { class: 'sr-only' }, t('tab.failures')),
      h('thead', null, h('tr', null, th('col.severity'), th('col.rule'), gidHead(), th('col.type'), th('col.name'), th('col.storey'), th('col.problem'), th('col.expected_actual', 'col-ea'), th('col.fix', 'col-fix'))),
      h('tbody', null, info.rows.map((f) => {
        const rule = ctx().rulesById.get(f.rule_id);
        return h('tr', { class: `is-clickable ${f === sel ? 'is-selected' : ''}`, 'data-fi': String(findingIndexOf(f)), onclick: () => actions.openDetail(f) },
          h('td', null, sevBadge(f.severity)),
          h('td', null, h('span', { class: 'mono', tip: rule ? rule.description : '' }, f.rule_id)),
          h('td', null, globalIdCell(f)),
          h('td', null, f.ifc_type || '–'),
          h('td', { class: 'wrap name' }, f.name || '–'),
          h('td', null, f.storey || '–'),
          h('td', { class: 'wrap' }, f.message),
          h('td', { class: 'wrap ea col-ea' },
            f.expected ? h('div', null, h('span', { class: 'ea-label' }, t('failures.expected')), f.expected) : null,
            f.actual ? h('div', null, h('span', { class: 'ea-label' }, t('failures.actual')), f.actual) : null),
          h('td', { class: 'wrap hint-cell col-fix' }, f.fix_hint || '–'));
      })))),
    pagerEl('failures', info));
}

export function updateSelectionHighlight() {
  const sel = state.detail && state.detail.finding;
  const idx = sel ? String(findingIndexOf(sel)) : null;
  $$('#tabpanel tr[data-fi]').forEach((tr) => tr.classList.toggle('is-selected', tr.dataset.fi === idx));
}

function sourcesTip(label, map) {
  const parts = Object.entries(map || {}).map(([k, n]) => `${n} ${t(`source.${k}.short`)}`);
  return parts.length ? `${label}: ${parts.join(' · ')}` : '';
}

function panelQuantities() {
  const q = ctx().report.qto;
  const rows = filteredQtoRows();
  const hasCost = q.rows.some((x) => x.cost);
  const cur = q.currency || (q.rows.find((x) => x.cost) || {}).cost?.currency || '';
  const parts = [];
  parts.push(h('div', { class: 'callout' },
    h('h3', null, t('qto.method_title')),
    h('p', null, q.method),
    q.elements_without_quantities
      ? h('p', { class: 'callout-warn' }, tn('qto.without_quantities', q.elements_without_quantities))
      : null));

  if (!rows.length) {
    parts.push(emptyState('empty.filters.title', 'empty.filters.body', clearButton()));
  } else {
    const info = pageSlice('quantities', rows);
    const th = (label, cls = '') => h('th', { scope: 'col', class: cls }, label);
    const num = (kind, v) => (v ? fmtQty(kind, v) : '–');
    const sub = rows.length !== q.rows.length
      ? rows.reduce((a, r) => ({ count: a.count + r.count, volume_m3: a.volume_m3 + r.volume_m3, area_m2: a.area_m2 + r.area_m2, length_m: a.length_m + r.length_m, weight_kg: a.weight_kg + r.weight_kg, cost: a.cost + (r.cost ? r.cost.amount : 0) }),
        { count: 0, volume_m3: 0, area_m2: 0, length_m: 0, weight_kg: 0, cost: 0 })
      : null;
    const totalRow = (labelKey, tot, cls) => h('tr', { class: `total-row ${cls}` },
      h('th', { scope: 'row', colspan: String(q.group_by.length) }, t(labelKey)),
      h('td', { class: 'num' }, nf(tot.count)),
      h('td', { class: 'num' }, num('vol', tot.volume_m3)),
      h('td', { class: 'num' }, num('area', tot.area_m2)),
      h('td', { class: 'num' }, num('len', tot.length_m)),
      h('td', { class: 'num' }, num('mass', tot.weight_kg)),
      hasCost ? h('td', null) : null,
      hasCost ? h('td', { class: 'num' }, tot.cost ? nf(tot.cost, 2) : '–') : null);
    parts.push(h('div', null,
      tableWrap(h('table', { class: 'data-table qto-table' },
        h('caption', { class: 'sr-only' }, t('tab.quantities')),
        h('thead', null, h('tr', null,
          q.group_by.map((g) => th(t(`group.${g}`))),
          th(t('qto.count'), 'num'),
          th(`${t('qto.volume')} (${unitLabel('vol')})`, 'num'),
          th(`${t('qto.area')} (${unitLabel('area')})`, 'num'),
          th(`${t('qto.length')} (${unitLabel('len')})`, 'num'),
          th(`${t('qto.weight')} (${unitLabel('mass')})`, 'num'),
          hasCost ? th(t('qto.rate'), 'num') : null,
          hasCost ? th(`${t('qto.cost')} (${cur})`, 'num') : null)),
        h('tbody', null, info.rows.map((r) => h('tr', null,
          r.group.map((g) => h('td', null, g || '–')),
          h('td', { class: 'num' }, nf(r.count)),
          h('td', { class: 'num', tip: sourcesTip(t('qto.volume_source'), r.volume_sources) }, num('vol', r.volume_m3)),
          h('td', { class: 'num', tip: sourcesTip(t('qto.area_source'), r.area_sources) }, num('area', r.area_m2)),
          h('td', { class: 'num', tip: sourcesTip(t('qto.length_source'), r.length_sources) }, num('len', r.length_m)),
          h('td', { class: 'num' }, num('mass', r.weight_kg)),
          hasCost ? h('td', { class: 'num rate', tip: r.cost ? `${r.cost.rate_code} · ${r.cost.rate_description} · ${r.cost.source}` : t('qto.no_rate') }, r.cost ? fmtRate(r.cost.rate, r.cost.unit, r.cost.currency) : '–') : null,
          hasCost ? h('td', { class: 'num strong' }, r.cost ? nf(r.cost.amount, 2) : '–') : null))),
        h('tfoot', null,
          sub ? totalRow('qto.selection', sub, 'selection') : null,
          totalRow('qto.total', { count: q.total_count, volume_m3: q.total_volume_m3, area_m2: q.total_area_m2, length_m: q.total_length_m, weight_kg: q.total_weight_kg, cost: q.total_cost }, '')))),
      pagerEl('quantities', info),
      h('p', { class: 'muted small tab-hint' }, t('qto.sources_hint'))));
  }

  // Hand-check samples
  const f = flt();
  const samples = q.samples.filter((s) => (!f.type || s.ifc_type === f.type) && (!f.storey || (f.storey === NONE_STOREY ? !s.storey : s.storey === f.storey)) && matchText(f.q, s.global_id, s.name));
  if (samples.length) {
    const cell = (kind, v, source) => h('td', { class: 'num' },
      v == null ? '–' : fmtQty(kind, v, 3),
      h('span', { class: `src src-${source}` }, t(`source.${source}.label`)));
    parts.push(h('section', { class: 'samples-block', 'aria-labelledby': 'samples-h' },
      h('h3', { id: 'samples-h' }, t('qto.samples_title')),
      h('p', { class: 'muted' }, t('qto.samples_hint')),
      tableWrap(h('table', { class: 'data-table samples-table' },
        h('caption', { class: 'sr-only' }, t('qto.samples_title')),
        h('thead', null, h('tr', null,
          [t('col.globalid'), t('col.type'), t('col.name'), t('col.storey')].map((x) => h('th', { scope: 'col' }, x)),
          h('th', { scope: 'col', class: 'num' }, `${t('qto.volume')} (${unitLabel('vol')})`),
          h('th', { scope: 'col', class: 'num' }, `${t('qto.area')} (${unitLabel('area')})`),
          h('th', { scope: 'col', class: 'num' }, `${t('qto.length')} (${unitLabel('len')})`),
          h('th', { scope: 'col' }, t('qto.qsets')))),
        h('tbody', null, samples.map((s) => h('tr', null,
          h('td', null, h('button', { type: 'button', class: 'linklike mono', onclick: () => actions.openDetail(null, { globalId: s.global_id }) }, s.global_id)),
          h('td', null, s.ifc_type), h('td', { class: 'wrap' }, s.name || '–'), h('td', null, s.storey || '–'),
          cell('vol', s.volume_m3, s.volume_source), cell('area', s.area_m2, s.area_source), cell('len', s.length_m, s.length_source),
          h('td', { class: 'wrap mono small' }, s.quantity_sets.length ? s.quantity_sets.join(', ') : t('qto.no_qsets'))))))))); 
  }
  return h('div', { class: 'stack' }, ...parts);
}

function panelIds() {
  const ids = ctx().report.ids;
  if (!ids) {
    return h('div', { class: 'empty empty-explain' },
      h('h3', null, t('ids.empty.title')),
      h('p', null, h('span', { class: 'term', tip: t('tip.ids') }, t('term.ids')), ' ', t('ids.empty.what')),
      h('p', null, t('ids.empty.example')),
      h('p', null, t('ids.empty.how')),
      h('button', { type: 'button', class: 'btn btn-secondary', onclick: () => actions.goto(1) }, t('review.change_settings')));
  }
  return h('div', { class: 'stack' },
    h('div', { class: 'ids-head' },
      h('div', null, h('h3', null, ids.title || ids.ids_file), h('p', { class: 'muted mono small' }, ids.ids_file)),
      h('span', { class: `badge ${ids.specifications_passed === ids.specifications_total ? 'st-pass' : 'st-fail'}` },
        t('ids.summary', { passed: ids.specifications_passed, total: ids.specifications_total }))),
    ...ids.specifications.map((spec, si) => idsSpec(spec, si)));
}

function idsSpec(spec, si) {
  const failedTotal = spec.requirements.reduce((a, r) => a + r.failed_entities.length, 0);
  return h('article', { class: 'card ids-spec' },
    h('header', { class: 'ids-spec-head' },
      h('div', null,
        h('h4', null, spec.name),
        spec.description ? h('p', { class: 'muted' }, spec.description) : null),
      h('div', { class: 'ids-badges' },
        spec.cardinality ? h('span', { class: 'chip' }, t(`ids.card.${spec.cardinality}`)) : null,
        h('span', { class: `badge ${spec.passed ? 'st-pass' : 'st-fail'}` }, spec.passed ? t('status.PASS') : t('status.FAIL')))),
    h('dl', { class: 'ids-meta' },
      h('div', null, h('dt', null, t('ids.applies_to')), h('dd', null, spec.applicability.join('; ') || '–')),
      h('div', null, h('dt', null, t('ids.applicable_count')), h('dd', null, nf(spec.applicable_count))),
      h('div', null, h('dt', null, t('ids.failed_count')), h('dd', { class: failedTotal ? 'fail-count' : '' }, nf(failedTotal)))),
    spec.instructions ? h('p', { class: 'ids-instructions' }, h('strong', null, t('ids.instructions')), ' ', spec.instructions) : null,
    h('ol', { class: 'ids-reqs' }, spec.requirements.map((req, ri) => idsReq(req, `${si}-${ri}`))));
}

function idsReq(req, key) {
  const limit = { n: 5 };
  const list = h('div', { class: 'ids-failed' });
  const draw = () => {
    clear(list);
    if (!req.failed_entities.length) return;
    const shownRows = req.failed_entities.slice(0, limit.n);
    list.appendChild(tableWrap(h('table', { class: 'data-table compact' },
      h('caption', { class: 'sr-only' }, t('ids.failed_entities')),
      h('thead', null, h('tr', null, [t('col.globalid'), t('col.type'), t('col.name'), t('ids.reason')].map((x) => h('th', { scope: 'col' }, x)))),
      h('tbody', null, shownRows.map((e) => h('tr', null,
        h('td', null, h('button', { type: 'button', class: 'linklike mono', onclick: () => actions.openDetail({ rule_id: `IDS: ${req.description}`, severity: 'ERROR', global_id: e.global_id, ifc_type: e.ifc_type, name: e.name, storey: null, message: e.reason, source: { kind: 'ids' } }) }, e.global_id)),
        h('td', null, e.ifc_type || '–'), h('td', { class: 'wrap' }, e.name || '–'), h('td', { class: 'wrap' }, e.reason)))))));
    if (req.failed_entities.length > limit.n) {
      list.appendChild(h('button', { type: 'button', class: 'btn btn-ghost', onclick: () => { limit.n += 25; draw(); } },
        t('ids.show_more', { shown: Math.min(limit.n, req.failed_entities.length), total: req.failed_entities.length })));
    }
  };
  draw();
  return h('li', { class: 'ids-req', 'data-key': key },
    h('div', { class: 'ids-req-head' },
      h('blockquote', null, req.description),
      h('div', { class: 'ids-badges' },
        req.facet ? h('span', { class: 'chip' }, tOr(`ids.facet.${req.facet}`, req.facet)) : null,
        h('span', { class: `badge ${req.passed ? 'st-pass' : 'st-fail'}` }, req.passed ? t('status.PASS') : t('ids.failed_n', { n: req.failed_entities.length })))),
    list);
}

function panelClash() {
  const c = ctx().report.clash;
  return h('div', { class: 'empty empty-explain' },
    h('span', { class: 'badge st-na' }, c.implemented ? t('clash.available') : t('clash.phase2')),
    h('h3', null, t('clash.title')),
    h('p', null, c.message),
    c.clashes && c.clashes.length ? h('p', { class: 'muted' }, tn('clash.found', c.clashes.length)) : null);
}

export function renderPanel() {
  const panel = $('#tabpanel');
  const tab = state.review.tab;
  panel.setAttribute('aria-labelledby', `tab-${tab}`);
  clear(panel);
  const body = { checks: panelChecks, failures: panelFailures, quantities: panelQuantities, ids: panelIds, clash: panelClash }[tab]();
  panel.appendChild(body);
}

export function renderReview() {
  if (!state.result) return;
  renderHead();
  renderTabs();
  renderFilterbar();
  renderPanel();
}

export function refreshHead() { if (state.result) renderHead(); }

// Close the verdict popover when clicking elsewhere.
document.addEventListener('click', (e) => {
  if (!e.target.closest || e.target.closest('.verdict-wrap')) return;
  closeVerdictPopover();
});
