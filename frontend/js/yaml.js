// A deliberately tiny YAML emitter for the project.yaml the settings panel
// edits: nested maps, inline lists, scalars. No parser, no library.

const RESERVED = new Set(['true', 'false', 'null', 'yes', 'no', 'on', 'off', '~', '']);

function numberText(n) {
  if (!Number.isFinite(n)) return '0';
  let s = String(n);
  if (/e/i.test(s)) s = n.toFixed(12).replace(/0+$/, '').replace(/\.$/, '');
  return s;
}

function scalar(v) {
  if (v === null || v === undefined) return 'null';
  if (typeof v === 'boolean') return v ? 'true' : 'false';
  if (typeof v === 'number') return numberText(v);
  if (Array.isArray(v)) return `[${v.map(scalar).join(', ')}]`;
  const s = String(v);
  const plain = /^[A-Za-z_][A-Za-z0-9_./ -]*$/.test(s) && !/\s$/.test(s) && !RESERVED.has(s.toLowerCase());
  return plain ? s : JSON.stringify(s);
}

const isMap = (v) => v && typeof v === 'object' && !Array.isArray(v);

export function emitYaml(obj, level = 0) {
  const pad = '  '.repeat(level);
  const out = [];
  for (const [k, v] of Object.entries(obj)) {
    if (v === undefined) continue;
    if (isMap(v)) {
      if (!Object.keys(v).length) out.push(`${pad}${k}: {}`);
      else { out.push(`${pad}${k}:`); out.push(emitYaml(v, level + 1)); }
    } else out.push(`${pad}${k}: ${scalar(v)}`);
  }
  return out.join('\n');
}
