// Display-only unit conversion. Engine data is always SI (m, m², m³, kg);
// nothing here changes what is stored, exported or sent to the engine.
import { state } from './state.js';
import { t } from './i18n.js';
import { nf } from './format.js';

const K = { len: 3.280839895013123, area: 10.763910416709722, vol: 35.31466672148859, mass: 2.2046226218487757 };
export const KINDS_BY_RATE_UNIT = { m: 'len', m2: 'area', m3: 'vol', kg: 'mass' };

export const unitLabel = (kind) => t(`unit.${state.units}.${kind}`);

export const toDisplay = (kind, v) => (state.units === 'imperial' ? v * K[kind] : v);

export function fmtQty(kind, v, digits = 2, withUnit = false) {
  if (v == null || Number.isNaN(v)) return '–';
  const s = nf(toDisplay(kind, v), digits);
  return withUnit ? `${s} ${unitLabel(kind)}` : s;
}

// A price per m³ becomes a price per ft³ when the display is imperial.
export function fmtRate(rate, rateUnit, currency) {
  const kind = KINDS_BY_RATE_UNIT[rateUnit];
  const shown = kind && state.units === 'imperial' ? rate / K[kind] : rate;
  const unit = kind ? unitLabel(kind) : t(`unit.rate.${rateUnit}`);
  return `${nf(shown, 2)} ${currency}/${unit}`;
}

// Map a quantity name from an IFC quantity set to a conversion kind.
export function kindForQuantityName(name) {
  const n = name.toLowerCase();
  if (n.includes('volume')) return 'vol';
  if (n.includes('area')) return 'area';
  if (n.includes('weight') || n.includes('mass')) return 'mass';
  if (/(length|height|width|depth|perimeter|thickness|diameter|radius)/.test(n)) return 'len';
  return null;
}
