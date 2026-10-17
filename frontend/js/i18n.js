// Minimal i18n: JSON dictionaries in i18n/<lang>.json, a t(key, args) lookup
// with {placeholder} substitution, plural helper tn() and a DOM applier for
// data-i18n attributes. English is always loaded as the fallback.
import { setTip } from './dom.js';

let base = {};
let dict = {};
let current = 'en';

export async function setLanguage(lang) {
  if (!Object.keys(base).length) base = await (await fetch('i18n/en.json')).json();
  dict = lang === 'en' ? base : await (await fetch(`i18n/${lang}.json`)).json();
  current = lang;
  document.documentElement.lang = lang;
}

export const getLanguage = () => current;
export const locale = () => (current === 'es' ? 'es-ES' : 'en-GB');

export function t(key, args) {
  let s = dict[key] ?? base[key] ?? key;
  if (args) for (const [k, v] of Object.entries(args)) s = s.split(`{${k}}`).join(String(v));
  return s;
}

// Plural helper: looks up key_one / key_other, always provides {n}.
export function tn(key, n, args) {
  return t(`${key}_${n === 1 ? 'one' : 'other'}`, { n: new Intl.NumberFormat(locale()).format(n), ...args });
}

export function applyI18n(root = document) {
  root.querySelectorAll('[data-i18n]').forEach((n) => { n.textContent = t(n.dataset.i18n); });
  root.querySelectorAll('[data-i18n-attr]').forEach((n) => {
    n.dataset.i18nAttr.split(';').forEach((pair) => {
      const [attr, key] = pair.split(':');
      if (attr && key) n.setAttribute(attr.trim(), t(key.trim()));
    });
  });
  root.querySelectorAll('[data-tip-key]').forEach((n) => setTip(n, t(n.dataset.tipKey)));
}
