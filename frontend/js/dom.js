// Tiny DOM helpers: element builder, SVG builder, tooltips and toasts.
// Text is always inserted with textContent / text nodes, never innerHTML,
// so file names, element names and engine messages cannot inject markup.

const PROPS = new Set(['value', 'checked', 'disabled', 'hidden', 'selected']);

function append(node, kids) {
  for (const k of kids) {
    if (k == null || k === false) continue;
    if (Array.isArray(k)) append(node, k);
    else if (k instanceof Node) node.appendChild(k);
    else node.appendChild(document.createTextNode(String(k)));
  }
}

export function h(tag, props, ...kids) {
  const n = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v == null || v === false) continue;
      if (k === 'class') n.className = v;
      else if (k === 'text') n.textContent = v;
      else if (k === 'tip') setTip(n, v);
      else if (k === 'dataset') Object.assign(n.dataset, v);
      else if (k.startsWith('on') && typeof v === 'function') n.addEventListener(k.slice(2), v);
      else if (PROPS.has(k)) n[k] = v;
      else if (v === true) n.setAttribute(k, '');
      else n.setAttribute(k, v);
    }
  }
  append(n, kids);
  return n;
}

const SVG_NS = 'http://www.w3.org/2000/svg';
export function svg(tag, props, ...kids) {
  const n = document.createElementNS(SVG_NS, tag);
  if (props) for (const [k, v] of Object.entries(props)) if (v != null) n.setAttribute(k, v);
  append(n, kids);
  return n;
}

export function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
  return node;
}

export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

// ---------------------------------------------------------------- tooltips

export function setTip(node, text) {
  if (!text) return;
  node.dataset.tip = text;
  if (!node.matches('button, a, input, select, textarea, [tabindex]')) node.tabIndex = 0;
}

let tipEl = null;
let tipTarget = null;

function showTip(target) {
  const text = target.dataset.tip;
  if (!text || !tipEl) return;
  tipTarget = target;
  tipEl.textContent = text;
  tipEl.hidden = false;
  target.setAttribute('aria-describedby', 'tooltip');
  const r = target.getBoundingClientRect();
  const w = tipEl.offsetWidth;
  const hgt = tipEl.offsetHeight;
  let top = r.bottom + 8;
  if (top + hgt > window.innerHeight - 8) top = Math.max(8, r.top - hgt - 8);
  let left = r.left + r.width / 2 - w / 2;
  left = Math.max(8, Math.min(left, window.innerWidth - w - 8));
  tipEl.style.top = `${top}px`;
  tipEl.style.left = `${left}px`;
}

export function hideTip() {
  if (!tipEl) return;
  tipEl.hidden = true;
  if (tipTarget) tipTarget.removeAttribute('aria-describedby');
  tipTarget = null;
}

export function initTooltips() {
  tipEl = h('div', { id: 'tooltip', role: 'tooltip', hidden: true });
  document.body.appendChild(tipEl);
  document.addEventListener('mouseover', (e) => {
    const t = e.target.closest ? e.target.closest('[data-tip]') : null;
    if (t) { if (t !== tipTarget) showTip(t); } else hideTip();
  });
  document.addEventListener('focusin', (e) => {
    const t = e.target.closest ? e.target.closest('[data-tip]') : null;
    if (t) showTip(t); else hideTip();
  });
  document.addEventListener('focusout', hideTip);
  document.addEventListener('scroll', hideTip, true);
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') hideTip(); });
}

// ------------------------------------------------------------------ toasts

export function toast(message, kind = 'info') {
  const host = document.getElementById('toasts');
  if (!host) return;
  const n = h('div', { class: `toast toast-${kind}` }, message);
  host.appendChild(n);
  setTimeout(() => n.remove(), 5200);
}
