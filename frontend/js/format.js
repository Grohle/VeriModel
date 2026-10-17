import { locale } from './i18n.js';

export const nf = (n, d = 0) =>
  new Intl.NumberFormat(locale(), { minimumFractionDigits: d, maximumFractionDigits: d }).format(n);

export function bytes(n) {
  if (n == null) return '–';
  if (n < 1024) return `${nf(n)} B`;
  if (n < 1024 * 1024) return `${nf(n / 1024, 1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${nf(n / 1024 / 1024, 1)} MB`;
  return `${nf(n / 1024 / 1024 / 1024, 2)} GB`;
}

export function dateTime(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return String(iso);
  return new Intl.DateTimeFormat(locale(), { dateStyle: 'medium', timeStyle: 'short' }).format(d);
}

export function duration(ms) {
  if (ms == null) return '–';
  return ms < 1000 ? `${nf(ms)} ms` : `${nf(ms / 1000, 1)} s`;
}

export const basename = (p) => String(p ?? '').split(/[\\/]/).pop();

export function joinPath(dir, name) {
  const sep = dir.includes('\\') && !dir.includes('/') ? '\\' : '/';
  return dir.replace(/[\\/]+$/, '') + sep + name;
}

export function extOf(p) {
  const m = /\.([A-Za-z0-9]+)$/.exec(String(p));
  return m ? m[1].toLowerCase() : '';
}
