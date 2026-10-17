// Pagination: tables never render more than one page of rows.
import { h } from './dom.js';
import { t } from './i18n.js';
import { nf } from './format.js';

export const PAGE_SIZES = [50, 100, 250];

export function paginate(items, page, size) {
  const pages = Math.max(1, Math.ceil(items.length / size));
  const p = Math.min(Math.max(1, page), pages);
  const start = (p - 1) * size;
  return { rows: items.slice(start, start + size), page: p, pages, start, end: Math.min(items.length, start + size), total: items.length };
}

export function pager(info, size, { onPage, onSize }) {
  const range = info.total
    ? t('pager.showing', { from: nf(info.start + 1), to: nf(info.end), total: nf(info.total) })
    : t('pager.none');
  return h('div', { class: 'pager' },
    h('span', { class: 'pager-range', role: 'status' }, range),
    h('div', { class: 'pager-controls' },
      h('label', { class: 'pager-size' }, t('pager.per_page'),
        h('select', { onchange: (e) => onSize(Number(e.target.value)) },
          PAGE_SIZES.map((n) => h('option', { value: String(n), selected: n === size }, String(n))))),
      h('button', { type: 'button', class: 'btn btn-ghost', disabled: info.page <= 1, onclick: () => onPage(info.page - 1) }, t('pager.prev')),
      h('span', { class: 'pager-page' }, t('pager.page', { page: nf(info.page), pages: nf(info.pages) })),
      h('button', { type: 'button', class: 'btn btn-ghost', disabled: info.page >= info.pages, onclick: () => onPage(info.page + 1) }, t('pager.next'))));
}
