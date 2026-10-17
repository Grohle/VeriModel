// Right-hand detail panel: the selected finding plus everything the engine
// knows about the element (properties, quantities, classifications, ...).
import { h, clear, $, toast } from './dom.js';
import { t, tn } from './i18n.js';
import { state, actions } from './state.js';
import * as api from './api.js';
import { nf } from './format.js';
import { fmtQty, kindForQuantityName, unitLabel } from './units.js';
import { sevBadge, updateSelectionHighlight } from './view-review.js';

export function isDetailOpen() { return Boolean(state.detail); }

export async function openDetail(finding, opts = {}) {
  const globalId = finding ? finding.global_id : opts.globalId;
  const opener = document.activeElement;
  state.detail = { finding, globalId, element: null, status: globalId ? 'loading' : 'none', error: null, opener };
  document.body.classList.add('detail-open');
  $('#detail').hidden = false;
  renderDetail();
  updateSelectionHighlight();
  $('#detail-close').focus();
  if (!globalId) return;
  const mine = state.detail;
  try {
    const el = await api.getElement(globalId);
    if (state.detail !== mine) return;
    mine.element = el; mine.status = 'ready';
  } catch (e) {
    if (state.detail !== mine) return;
    mine.status = 'error'; mine.error = e;
  }
  renderDetail();
}

export function closeDetail() {
  const d = state.detail;
  if (!d) return false;
  state.detail = null;
  document.body.classList.remove('detail-open');
  $('#detail').hidden = true;
  updateSelectionHighlight();
  if (d.opener && document.contains(d.opener) && typeof d.opener.focus === 'function') d.opener.focus();
  return true;
}

const dash = '–';

function propText(v) {
  if (v === null || v === undefined) return h('span', { class: 'empty-val' }, t('detail.null'));
  if (typeof v === 'boolean') return v ? t('common.yes') : t('common.no');
  if (typeof v === 'number') return nf(v, Number.isInteger(v) ? 0 : 3);
  if (Array.isArray(v)) return v.length ? v.map((x) => (typeof x === 'object' ? JSON.stringify(x) : String(x))).join(', ') : h('span', { class: 'empty-val' }, t('detail.empty'));
  if (String(v).trim() === '') return h('span', { class: 'empty-val' }, t('detail.empty'));
  return String(v);
}

function kv(rows) {
  return h('dl', { class: 'kv' }, rows.map(([k, v]) => h('div', null, h('dt', null, k), h('dd', null, v))));
}

function section(titleNode, ...kids) {
  return h('section', { class: 'd-sec' }, h('h3', null, titleNode), ...kids);
}

function findingBlock(f) {
  const rule = state.result.report.rules.find((r) => r.rule_id === f.rule_id);
  return h('section', { class: `d-finding sev-${f.severity.toLowerCase()}` },
    h('div', { class: 'd-finding-head' }, sevBadge(f.severity), h('span', { class: 'mono' }, f.rule_id),
      rule ? h('span', { class: 'muted' }, rule.description) : null),
    h('p', { class: 'd-message' }, f.message),
    f.expected || f.actual ? kv([[t('failures.expected'), f.expected || dash], [t('failures.actual'), f.actual || dash]]) : null,
    f.fix_hint ? h('p', { class: 'd-fix' }, h('strong', null, t('detail.fix')), ' ', f.fix_hint) : null);
}

function geometryBlock(el) {
  const g = el.geometry;
  if (!g) return h('p', { class: 'muted' }, el.has_representation ? t('detail.geom.none') : t('detail.geom.no_rep'));
  const len = (v) => fmtQty('len', v, 3, true);
  const ext = g.bbox_max.map((v, i) => v - g.bbox_min[i]);
  const c = g.bbox_min.map((v, i) => (v + g.bbox_max[i]) / 2);
  const dist = Math.sqrt(c[0] ** 2 + c[1] ** 2 + c[2] ** 2);
  return kv([
    [t('detail.geom.min'), g.bbox_min.map((v) => fmtQty('len', v, 2)).join(' , ')],
    [t('detail.geom.max'), g.bbox_max.map((v) => fmtQty('len', v, 2)).join(' , ')],
    [t('detail.geom.size'), `${ext.map((v) => fmtQty('len', v, 2)).join(' × ')} ${unitLabel('len')}`],
    [t('detail.geom.distance'), len(dist)],
    [t('qto.volume'), g.volume_m3 == null ? dash : fmtQty('vol', g.volume_m3, 3, true)],
    [t('qto.area'), g.area_m2 == null ? dash : fmtQty('area', g.area_m2, 3, true)],
    [t('detail.geom.triangles'), nf(g.triangle_count)],
  ]);
}

function elementBlocks(el) {
  const out = [];
  out.push(section(t('detail.sec.identity'), kv([
    [h('span', { class: 'term', tip: t('tip.globalid') }, t('term.globalid')), h('span', { class: 'copy-row' }, h('span', { class: 'mono' }, el.global_id),
      h('button', { type: 'button', class: 'btn btn-ghost btn-sm', onclick: async () => {
        try { await navigator.clipboard.writeText(el.global_id); toast(t('detail.copied'), 'success'); } catch { toast(t('detail.copy_failed'), 'error'); }
      } }, t('detail.copy')))],
    [t('detail.step_id'), `#${el.step_id}`],
    [t('col.type'), el.ifc_type + (el.predefined_type ? ` · ${el.predefined_type}` : '')],
    [t('col.name'), el.name || dash],
    [t('detail.type_name'), el.type_name || dash],
    [t('detail.tag'), el.tag || dash],
    [t('col.storey'), el.storey || dash],
    [t('detail.container'), el.container ? `${el.container.ifc_type}${el.container.name ? ` · ${el.container.name}` : ''}` : t('detail.no_container')],
    [t('detail.materials'), el.materials && el.materials.length ? el.materials.join(', ') : dash],
  ])));

  const psets = Object.entries(el.psets || {});
  out.push(section(h('span', null, h('span', { class: 'term', tip: t('tip.pset') }, t('term.pset')), ` · ${nf(psets.length)}`),
    psets.length ? psets.map(([name, props], i) => h('details', { class: 'pset', open: i < 2 },
      h('summary', null, h('span', { class: 'mono' }, name), h('span', { class: 'muted' }, ` ${nf(Object.keys(props).length)}`)),
      kv(Object.entries(props).map(([k, v]) => [k, propText(v)]))))
      : h('p', { class: 'muted' }, t('detail.no_psets'))));

  const qsets = Object.entries(el.quantities || {});
  out.push(section(t('detail.sec.quantities'),
    qsets.length ? qsets.map(([name, qs]) => h('details', { class: 'pset', open: true },
      h('summary', null, h('span', { class: 'mono' }, name)),
      kv(Object.entries(qs).map(([k, v]) => {
        const kind = kindForQuantityName(k);
        return [k, kind ? `${fmtQty(kind, v, 3)} ${unitLabel(kind)}` : nf(v, 3)];
      })))) : h('p', { class: 'muted' }, t('detail.no_quantities'))));

  const cls = el.classifications || [];
  out.push(section(t('detail.sec.classifications'),
    cls.length ? h('ul', { class: 'plain' }, cls.map((c) => h('li', null, h('span', { class: 'mono' }, c.code || dash), ' ', c.name || '', h('span', { class: 'muted' }, c.system ? ` · ${c.system}` : ''))))
      : h('p', { class: 'muted' }, t('detail.no_classifications'))));

  out.push(section(t('detail.sec.geometry'), geometryBlock(el)));
  return out;
}

export function renderDetail() {
  const d = state.detail;
  const root = $('#detail-body');
  clear(root);
  if (!d) return;
  const f = d.finding;
  const el = d.element;
  const title = (el && el.name) || (f && f.name) || (f && f.global_id) || d.globalId || t('detail.model_level');
  const subtitle = (el && el.ifc_type) || (f && f.ifc_type) || '';
  $('#detail-title').textContent = title;
  $('#detail-sub').textContent = subtitle;

  if (f) root.appendChild(findingBlock(f));
  if (d.status === 'loading') root.appendChild(h('div', { class: 'skeleton-stack', role: 'status' }, h('div', { class: 'skeleton w60' }), h('div', { class: 'skeleton w80' }), h('div', { class: 'skeleton w40' }), h('p', { class: 'muted' }, t('detail.loading'))));
  else if (d.status === 'none') root.appendChild(h('p', { class: 'muted' }, t('detail.model_level_body')));
  else if (d.status === 'error') root.appendChild(h('div', { class: 'notice notice-warning' }, h('div', { class: 'notice-body' }, h('strong', null, t('detail.unavailable')), h('p', null, d.error.message), d.error.hint ? h('p', { class: 'muted' }, d.error.hint) : null)));
  else if (el) root.append(...elementBlocks(el));

  // other findings on the same element
  if (d.globalId) {
    const others = state.result.report.findings.filter((x) => x.global_id === d.globalId && x !== f);
    if (others.length) {
      root.appendChild(section(tn('detail.other_findings', others.length),
        h('ul', { class: 'plain other-findings' }, others.map((o) => h('li', null,
          h('button', { type: 'button', class: 'linklike', onclick: () => actions.openDetail(o) }, h('span', { class: 'mono' }, o.rule_id), ' ', o.message),
          ' ', sevBadge(o.severity))))));
    }
  }
}

export function initDetail() {
  $('#detail-close').addEventListener('click', closeDetail);
}
