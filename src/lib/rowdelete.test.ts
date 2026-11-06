import { describe, expect, it } from 'vitest';
import { deletability, keyIndexes, keyValues, rowsLabel, sourceTableOid } from './rowdelete';
import type { Column, RowIdentity } from './types';

const col = (name: string, tableOid: number | null, columnId: number | null): Column => ({
  name,
  typeName: 'int4',
  sortKind: 'number',
  tableOid,
  columnId,
});

// SELECT id, label FROM items
const fromItems = [col('id', 42, 1), col('label', 42, 2)];

const identity: RowIdentity = {
  schema: 'shop',
  table: 'items',
  qualified: 'shop.items',
  key: [{ name: 'id', quoted: 'id', attnum: 1 }],
};
const composite: RowIdentity = {
  schema: 'shop',
  table: 'pairs',
  qualified: 'shop.pairs',
  key: [
    { name: 'a', quoted: 'a', attnum: 1 },
    { name: 'b', quoted: 'b', attnum: 2 },
  ],
};

describe('sourceTableOid', () => {
  it('finds the table when every column came from it', () => {
    expect(sourceTableOid(fromItems)).toBe(42);
  });
  it('is null when the columns disagree, as in a join', () => {
    expect(sourceTableOid([col('a', 42, 1), col('b', 43, 1)])).toBeNull();
  });
  it('is null when any column is computed', () => {
    expect(sourceTableOid([col('id', 42, 1), col('count', null, null)])).toBeNull();
    expect(sourceTableOid([col('count', null, null)])).toBeNull();
  });
  it('is null for an empty result', () => {
    expect(sourceTableOid([])).toBeNull();
  });
});

describe('keyIndexes', () => {
  it('finds the key by position in the table, not by name', () => {
    // SELECT label, id AS whatever FROM items
    const renamed = [col('label', 42, 2), col('whatever', 42, 1)];
    expect(keyIndexes(renamed, identity)).toEqual([1]);
  });
  it('keeps the key order for a composite key', () => {
    const swapped = [col('b', 42, 2), col('note', 42, 3), col('a', 42, 1)];
    expect(keyIndexes(swapped, composite)).toEqual([2, 0]);
  });
  it('is null when the query left a key column out', () => {
    expect(keyIndexes([col('label', 42, 2)], identity)).toBeNull();
    expect(keyIndexes([col('a', 42, 1), col('note', 42, 3)], composite)).toBeNull();
  });
});

describe('deletability', () => {
  const ok = (d: ReturnType<typeof deletability>) => d.ok;

  it('allows it when one table, a key in the result and rows selected', () => {
    expect(ok(deletability(fromItems, identity, null, [0], false))).toBe(true);
  });

  it('refuses while the query is still running', () => {
    const d = deletability(fromItems, identity, null, [0], true);
    expect(d.ok).toBe(false);
    expect(d.reason).toMatch(/finish/);
  });

  it('refuses a join and says why', () => {
    const d = deletability([col('a', 42, 1), col('b', 43, 1)], identity, null, [0], false);
    expect(d.ok).toBe(false);
    expect(d.reason).toMatch(/single table/);
  });

  it('passes on the server reason, such as a view or a missing primary key', () => {
    const d = deletability(fromItems, null, 'This table has no primary key.', [0], false);
    expect(d.ok).toBe(false);
    expect(d.reason).toBe('This table has no primary key.');
  });

  it('names the columns to add when the key is not in the result', () => {
    const d = deletability([col('label', 42, 2)], composite, null, [0], false);
    expect(d.ok).toBe(false);
    expect(d.reason).toContain('a, b');
  });

  it('asks for a selection when nothing is selected', () => {
    const d = deletability(fromItems, identity, null, [], false);
    expect(d.ok).toBe(false);
    expect(d.reason).toMatch(/row number/);
  });

  it('refuses an empty result', () => {
    expect(ok(deletability([], identity, null, [0], false))).toBe(false);
  });
});

describe('keyValues', () => {
  const rows = [
    ['1', 'one'],
    ['2', 'two'],
    ['3', 'three'],
  ];

  it('takes the key values of the chosen data rows, in key order', () => {
    expect(keyValues(rows, [0, 2], [0])).toEqual([['1'], ['3']]);
  });

  it('follows the key column order, not the column order', () => {
    const pairs = [['x', '7', 'note']];
    expect(keyValues(pairs, [0], [1, 0])).toEqual([['7', 'x']]);
  });

  it('refuses a NULL key rather than building a statement that matches nothing', () => {
    expect(keyValues([[null, 'one']], [0], [0])).toBeNull();
  });

  it('refuses a row that is not there', () => {
    expect(keyValues(rows, [9], [0])).toBeNull();
  });
});

describe('rowsLabel', () => {
  it('counts in words a person would use', () => {
    expect(rowsLabel(1)).toBe('1 row');
    expect(rowsLabel(2)).toBe('2 rows');
    expect(rowsLabel(1500)).toBe('1,500 rows');
  });
});
