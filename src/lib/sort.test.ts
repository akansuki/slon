import { describe, expect, it } from 'vitest';
import {
  compareDecimal,
  compareNumeric,
  effectiveKind,
  nextSort,
  sortedOrder,
} from './sort';
import type { Cell } from './types';

const col = (values: Cell[]): Cell[][] => values.map((v) => [v]);
const sortValues = (values: Cell[], kind: 'number' | 'text' | 'boolean' | 'unknown', dir: 'asc' | 'desc') => {
  const rows = col(values);
  return Array.from(sortedOrder(rows, 0, kind, dir), (i) => rows[i][0]);
};

describe('compareDecimal', () => {
  it('compares exactly beyond double precision', () => {
    expect(compareDecimal('9007199254740993', '9007199254740992')).toBe(1);
    expect(compareDecimal('0.10000000000000000001', '0.1')).toBe(1);
  });
  it('ignores trailing zeros and the sign of zero', () => {
    expect(compareDecimal('0.10', '0.1')).toBe(0);
    expect(compareDecimal('-0', '0')).toBe(0);
    expect(compareDecimal('-0.0', '0.00')).toBe(0);
  });
  it('orders negatives and fractions', () => {
    expect(compareDecimal('-2', '-10')).toBe(1);
    expect(compareDecimal('1.5', '1.25')).toBe(1);
    expect(compareDecimal('-1.5', '-1.25')).toBe(-1);
    expect(compareDecimal('100', '99.99')).toBe(1);
  });
});

describe('compareNumeric', () => {
  it('orders the non-finite values like PostgreSQL', () => {
    expect(compareNumeric('-Infinity', '-1e308')).toBe(-1);
    expect(compareNumeric('Infinity', '1e308')).toBe(1);
    expect(compareNumeric('NaN', 'Infinity')).toBe(1);
    expect(compareNumeric('NaN', 'NaN')).toBe(0);
  });
  it('handles scientific notation from float columns', () => {
    expect(compareNumeric('1e+20', '99999')).toBe(1);
    expect(compareNumeric('1.5e-3', '0.001')).toBe(1);
  });
});

describe('sortedOrder', () => {
  it('sorts numbers numerically, NULL last ascending and first descending', () => {
    const v = ['10', '9', '-1', '2.5', null, 'NaN', 'Infinity'];
    expect(sortValues(v, 'number', 'asc')).toEqual(['-1', '2.5', '9', '10', 'Infinity', 'NaN', null]);
    expect(sortValues(v, 'number', 'desc')).toEqual([null, 'NaN', 'Infinity', '10', '9', '2.5', '-1']);
  });

  it('keeps bigint precision', () => {
    const v = ['9007199254740994', '9007199254740992', '9007199254740993', '5'];
    expect(sortValues(v, 'number', 'asc')).toEqual(['5', '9007199254740992', '9007199254740993', '9007199254740994']);
  });

  it('is stable for equal values', () => {
    const rows: Cell[][] = [['1', 'a'], ['1', 'b'], ['0', 'c'], ['1', 'd']];
    const order = Array.from(sortedOrder(rows, 0, 'number', 'asc'));
    expect(order.map((i) => rows[i][1])).toEqual(['c', 'a', 'b', 'd']);
    const desc = Array.from(sortedOrder(rows, 0, 'number', 'desc'));
    expect(desc.map((i) => rows[i][1])).toEqual(['a', 'b', 'd', 'c']);
  });

  it('sorts text with NULL last', () => {
    expect(sortValues(['b', null, 'a', 'C'], 'text', 'asc')).toEqual(['a', 'b', 'C', null]);
  });

  it('sorts booleans f before t', () => {
    expect(sortValues(['t', 'f', null, 't'], 'boolean', 'asc')).toEqual(['f', 't', 't', null]);
  });

  it('copes with a large column', () => {
    const rows = col(Array.from({ length: 100_000 }, (_, i) => String((i * 7919) % 100_003)));
    const t0 = performance.now();
    const order = sortedOrder(rows, 0, 'number', 'asc');
    const ms = performance.now() - t0;
    for (let i = 1; i < order.length; i++) {
      expect(Number(rows[order[i - 1]][0])).toBeLessThanOrEqual(Number(rows[order[i]][0]));
    }
    expect(ms).toBeLessThan(2000);
  });
});

describe('effectiveKind', () => {
  it('infers numbers for untyped columns only when every value is numeric', () => {
    expect(effectiveKind('unknown', col(['1', '2.5', null, '1e3']), 0)).toBe('number');
    expect(effectiveKind('unknown', col(['1', 'x']), 0)).toBe('text');
    expect(effectiveKind('unknown', col([null, null]), 0)).toBe('text');
    expect(effectiveKind('text', col(['1', '2']), 0)).toBe('text');
  });
});

describe('nextSort', () => {
  it('cycles asc, desc, none and restarts on another column', () => {
    const a = nextSort(null, 2);
    expect(a).toEqual({ col: 2, dir: 'asc' });
    const b = nextSort(a, 2);
    expect(b).toEqual({ col: 2, dir: 'desc' });
    expect(nextSort(b, 2)).toBeNull();
    expect(nextSort(b, 0)).toEqual({ col: 0, dir: 'asc' });
  });
});
