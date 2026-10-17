// Step 3: downloads, BCF summary, run history with run-to-run diff.
import { h, clear, $ } from './dom.js';
import { t, tn } from './i18n.js';
import { state, actions } from './state.js';
import * as api from './api.js';
import { bytes, nf, dateTime, basename, joinPath } from './format.js';
import { paginate, pager } from './table.js';

// Display order of the output kinds; unknown kinds go last.
const KIND_ORDER = ['report_html', 'failures_csv', 'bcf', 'issues_csv', 'qto_xlsx', 'qto_csv', 'qto_json', 'report_json', 'config', 'run_json', 'audit_log'];
const VERDICT_CLASS = { PASS: 'v-pass', PASS_WITH_WARNINGS: 'v-warn', FAIL: 'v-fail' };
let histPage = 1;

async function open(path) {
  try {
    const ok = await api.openPath(path);
    if (!ok) actions.toast(t('export.browser_open', { path }), 'info');
  } catch (e) {
    actions.toast(`${e.message}${e.hint ? ` ${e.hint}` : ''}`, 'error');
  }
}

function kindRank(k) { const i = KIND_ORDER.indexOf(k); return i < 0 ? 99 : i; }

function fileRow(f, outDir, bcf) {
  const known = KIND_ORDER.includes(f.kind);
  const title = known ? t(`export.kind.${f.kind}.title`) : f.name;
  const desc = known ? t(`export.kind.${f.kind}.desc`) : t('export.kind.other.desc');
  const bcfLine = f.kind === 'bcf' && bcf
    ? h('p', { class: 'file-extra' }, t('export.bcf_line', { topics: nf(bcf.topics), viewpoints: nf(bcf.with_viewpoint), version: bcf.version }))
    : null;
  return h('li', { class: 'download' },
    h('div', { class: 'download-main' },
      h('h3', null, f.kind === 'bcf' ? h('span', { class: 'term', tip: t('tip.bcf') }, title) : title),
      h('p', null, desc),
      bcfLine,
      h('p', { class: 'file-name mono' }, f.name)),
    h('span', { class: 'file-size' }, bytes(f.bytes)),
    h('button', { type: 'button', class: 'btn btn-secondary', 'aria-label': t('export.open_named', { name: f.name }), onclick: () => open(joinPath(outDir, f.name)) }, t('export.open')));
}

export function diffFor(runs, i) {
  const run = runs[i];
  const prev = runs.slice(i + 1).find((r) => basename(r.ifc_file) === basename(run.ifc_file));
  if (!prev) return { text: t('hist.first'), cls: 'muted' };
  const dE = run.errors - prev.errors;
  const dW = run.warnings - prev.warnings;
  if (dE < 0) return { text: tn('hist.fewer_errors', -dE), cls: 'good' };
  if (dE > 0) return { text: tn('hist.more_errors', dE), cls: 'bad' };
  if (dW < 0) return { text: tn('hist.fewer_warnings', -dW), cls: 'good' };
  if (dW > 0) return { text: tn('hist.more_warnings', dW), cls: 'bad' };
  return { text: t('hist.same'), cls: 'muted' };
}

function historyTable() {
  const runs = state.runs;
  if (state.runsError) return h('div', { class: 'notice notice-warning' }, h('div', { class: 'notice-body' }, h('strong', null, t('hist.error')), h('p', null, state.runsError.message)));
  if (!runs.length) return h('p', { class: 'muted' }, t('hist.empty'));
  const info = paginate(runs, histPage, state.review.pageSize);
  histPage = info.page;
  const current = state.result.ledger.timestamp;
  const th = (k, cls = '') => h('th', { scope: 'col', class: cls }, t(k));
  return h('div', null,
    h('div', { class: 'table-wrap' }, h('table', { class: 'data-table history-table' },
      h('caption', { class: 'sr-only' }, t('hist.title')),
      h('thead', null, h('tr', null, th('hist.date'), th('hist.file'), th('hist.score', 'num'), th('hist.verdict'), th('hist.errors', 'num'), th('hist.warnings', 'num'), th('hist.change'), th('hist.folder'))),
      h('tbody', null, info.rows.map((r) => {
        const idx = runs.indexOf(r);
        const d = diffFor(runs, idx);
        return h('tr', { class: r.timestamp === current ? 'is-current' : '' },
          h('td', null, dateTime(r.timestamp), r.timestamp === current ? h('span', { class: 'chip chip-accent' }, t('hist.this_run')) : null),
          h('td', { class: 'wrap' }, basename(r.ifc_file)),
          h('td', { class: 'num strong' }, String(r.score)),
          h('td', null, h('span', { class: `badge verdict ${VERDICT_CLASS[r.verdict] || 'v-none'}` }, t(`verdict.${r.verdict || 'NONE'}`))),
          h('td', { class: 'num' }, nf(r.errors)),
          h('td', { class: 'num' }, nf(r.warnings)),
          h('td', { class: `diff ${d.cls}` }, d.text),
          h('td', null, h('button', { type: 'button', class: 'btn btn-ghost', 'aria-label': t('hist.open_named', { date: dateTime(r.timestamp) }), onclick: () => open(r.output_dir) }, t('hist.open'))));
      })))),
    pager(info, state.review.pageSize, {
      onPage: (p) => { histPage = p; renderExport(); },
      onSize: (n) => { state.review.pageSize = n; histPage = 1; renderExport(); },
    }));
}

export function renderExport() {
  const root = $('#export-root');
  clear(root);
  const res = state.result;
  if (!res) return;
  const { report, files, bcf, out_dir } = res;
  const s = report.summary;
  const sorted = [...files].sort((a, b) => kindRank(a.kind) - kindRank(b.kind));
  root.append(
    h('div', { class: 'export-top card' },
      h('div', null,
        h('p', { class: 'eyebrow' }, t('export.done')),
        h('h2', null, report.model.file_name),
        h('p', { class: 'muted' }, t('export.saved_to')),
        h('p', { class: 'mono out-dir' }, out_dir)),
      h('div', { class: 'export-top-actions' },
        h('span', { class: `badge verdict ${VERDICT_CLASS[s.verdict] || 'v-none'}` }, `${t(`verdict.${s.verdict || 'NONE'}`)} · ${s.score}`),
        h('button', { type: 'button', class: 'btn btn-secondary', onclick: () => open(out_dir) }, t('export.open_folder')))),
    h('section', { 'aria-labelledby': 'dl-title' },
      h('div', { class: 'section-head' }, h('h2', { id: 'dl-title' }, t('export.downloads')), h('p', { class: 'muted' }, t('export.downloads_sub'))),
      h('ul', { class: 'downloads' }, sorted.map((f) => fileRow(f, out_dir, bcf)))),
    h('section', { 'aria-labelledby': 'hist-title' },
      h('div', { class: 'section-head' }, h('h2', { id: 'hist-title' }, t('hist.title')), h('p', { class: 'muted' }, t('hist.sub'))),
      historyTable()),
    h('div', { class: 'export-footer' },
      h('button', { type: 'button', class: 'btn btn-secondary', onclick: () => actions.goto(2) }, t('export.back_review')),
      h('button', { type: 'button', class: 'btn btn-primary', onclick: () => actions.newCheck() }, t('export.new_check'))));
}
