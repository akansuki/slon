import type { Cell, SortKind } from './types';

export type SortDir = 'asc' | 'desc';
export interface SortSpec {
  col: number;
  dir: SortDir;
}

// Values arrive as the text PostgreSQL formatted, so every comparison starts from a string.
// NULL sorts as the largest value (last ascending, first descending), like PostgreSQL.

const PLAIN_DECIMAL = /^[+-]?\d+(\.\d+)?$/;
const NUMBERISH = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/;
const SPECIAL = new Set(['NaN', 'Infinity', '-Infinity', '+Infinity']);

/** Exact comparison of two plain decimal strings ("-12.50"), any length. */
export function compareDecimal(a: string, b: string): number {
  const pa = parseDecimal(a);
  const pb = parseDecimal(b);
  if (pa.neg !== pb.neg) return pa.neg ? -1 : 1;
  let c = pa.int.length - pb.int.length;
  if (c === 0) c = pa.int < pb.int ? -1 : pa.int > pb.int ? 1 : 0;
  if (c === 0) {
    const len = Math.max(pa.frac.length, pb.frac.length);
    const fa = pa.frac.padEnd(len, '0');
    const fb = pb.frac.padEnd(len, '0');
    c = fa < fb ? -1 : fa > fb ? 1 : 0;
  }
  return pa.neg ? -c : c;
}

function parseDecimal(s: string) {
  const neg = s.startsWith('-');
  const body = s.replace(/^[+-]/, '');
  const dot = body.indexOf('.');
  const int = (dot < 0 ? body : body.slice(0, dot)).replace(/^0+(?=\d)/, '');
  const frac = (dot < 0 ? '' : body.slice(dot + 1)).replace(/0+$/, '');
  const zero = int === '0' && frac === '';
  return { neg: neg && !zero, int, frac };
}

/** Rank for the non-finite values: -Infinity < finite < Infinity < NaN (PostgreSQL order). */
function specialRank(s: string): number {
  if (s === '-Infinity') return -1;
  if (s === 'Infinity' || s === '+Infinity') return 1;
  if (s === 'NaN') return 2;
  return 0;
}

export function compareNumeric(a: string, b: string): number {
  const sa = SPECIAL.has(a);
  const sb = SPECIAL.has(b);
  if (sa || sb) {
    const ra = sa ? specialRank(a) : 0;
    const rb = sb ? specialRank(b) : 0;
    return ra < rb ? -1 : ra > rb ? 1 : 0;
  }
  if (PLAIN_DECIMAL.test(a) && PLAIN_DECIMAL.test(b)) return compareDecimal(a, b);
  const na = Number(a);
  const nb = Number(b);
  if (Number.isNaN(na) || Number.isNaN(nb)) return a < b ? -1 : a > b ? 1 : 0;
  return na < nb ? -1 : na > nb ? 1 : 0;
}

const collator = new Intl.Collator(undefined, { sensitivity: 'variant' });

export function compareText(a: string, b: string): number {
  return collator.compare(a, b);
}

/** Booleans arrive as 't' / 'f'. */
export function compareBool(a: string, b: string): number {
  return (a === 't' ? 1 : 0) - (b === 't' ? 1 : 0);
}

/**
 * Columns without type information (multi-statement scripts) are treated as numbers
 * when every non-null value looks like one, otherwise as text.
 */
export function effectiveKind(kind: SortKind, rows: Cell[][], col: number): SortKind {
  if (kind !== 'unknown') return kind;
  let sawValue = false;
  for (const row of rows) {
    const v = row[col];
    if (v === null) continue;
    sawValue = true;
    if (!NUMBERISH.test(v) && !SPECIAL.has(v)) return 'text';
  }
  return sawValue ? 'number' : 'text';
}

/**
 * Returns the row indices in display order without moving or copying the rows.
 * The sort is stable, so equal values keep the server's order.
 */
export function sortedOrder(rows: Cell[][], col: number, kind: SortKind, dir: SortDir): Uint32Array {
  const k = effectiveKind(kind, rows, col);
  const compare = k === 'number' ? compareNumeric : k === 'boolean' ? compareBool : compareText;

  // Fast path for numbers: plain decimals of up to 15 significant digits map to
  // doubles without losing order, so they can be compared as numbers.
  let fast: Float64Array | null = null;
  let isFast: Uint8Array | null = null;
  if (k === 'number') {
    fast = new Float64Array(rows.length);
    isFast = new Uint8Array(rows.length);
    for (let i = 0; i < rows.length; i++) {
      const v = rows[i][col];
      if (v !== null && PLAIN_DECIMAL.test(v) && v.replace(/[^\d]/g, '').replace(/^0+/, '').length <= 15) {
        fast[i] = Number(v);
        isFast[i] = 1;
      }
    }
  }

  const order = new Uint32Array(rows.length);
  for (let i = 0; i < order.length; i++) order[i] = i;

  const sign = dir === 'asc' ? 1 : -1;
  order.sort((x, y) => {
    const a = rows[x][col];
    const b = rows[y][col];
    if (a === null || b === null) {
      if (a === b) return 0;
      return (a === null ? 1 : -1) * sign;
    }
    if (fast && isFast![x] && isFast![y]) {
      const d = fast[x] - fast[y];
      return d === 0 ? 0 : d < 0 ? -sign : sign;
    }
    return compare(a, b) * sign;
  });
  return order;
}

/** asc -> desc -> unsorted, the usual header-click cycle. */
export function nextSort(current: SortSpec | null, col: number): SortSpec | null {
  if (!current || current.col !== col) return { col, dir: 'asc' };
  return current.dir === 'asc' ? { col, dir: 'desc' } : null;
}
