import { describe, expect, it } from 'vitest';
import { deletability, editableColumns, keyIndexes, keyValues, rowsLabel, savedMessage, sourceTableOid } from './griddata';
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
  columns: [
    { name: 'id', quoted: 'id', attnum: 1, editable: true, reason: null },
    { name: 'label', quoted: 'label', attnum: 2, editable: true, reason: null },
    { name: 'doubled', quoted: 'doubled', attnum: 3, editable: false, reason: 'a generated column' },
  ],
};
const composite: RowIdentity = {
  schema: 'shop',
  table: 'pairs',
  qualified: 'shop.pairs',
  key: [
    { name: 'a', quoted: 'a', attnum: 1 },
    { name: 'b', quoted: 'b', attnum: 2 },
  ],
  columns: [
    { name: 'a', quoted: 'a', attnum: 1, editable: true, reason: null },
    { name: 'b', quoted: 'b', attnum: 2, editable: true, reason: null },
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

describe('editableColumns', () => {
  it('allows the stored columns of the source table', () => {
    const v = editableColumns(fromItems, identity);
    expect(v.map((x) => x.ok)).toEqual([true, true]);
  });

  it('refuses a generated column and names it', () => {
    const cols = [col('id', 42, 1), col('doubled', 42, 3)];
    const v = editableColumns(cols, identity);
    expect(v[1].ok).toBe(false);
    expect(v[1].reason).toBe('doubled is a generated column');
  });

  it('refuses a computed column, which has no position in the table', () => {
    const cols = [col('id', 42, 1), col('twice', 42, null)];
    const v = editableColumns(cols, identity);
    expect(v[1].ok).toBe(false);
    expect(v[1].reason).toMatch(/computed/);
  });

  it('makes the whole result read-only when the key is missing', () => {
    const v = editableColumns([col('label', 42, 2)], identity);
    expect(v.every((x) => !x.ok)).toBe(true);
    expect(v[0].reason).toContain('(id)');
  });

  it('makes the whole result read-only when there is no identity at all', () => {
    const v = editableColumns(fromItems, null);
    expect(v.every((x) => !x.ok)).toBe(true);
  });
});

describe('savedMessage', () => {
  it('confirms the write when the value went in as typed', () => {
    expect(savedMessage('label', 'hi', 'hi', 4)).toBe('Saved label in 4 ms');
  });
  it('says what was stored when the server rendered it differently', () => {
    expect(savedMessage('price', '1.5', '1.50', 4)).toBe('Saved price as 1.50');
  });
  it('names NULL rather than showing nothing', () => {
    expect(savedMessage('label', '', null, 4)).toBe('Saved label as NULL');
    expect(savedMessage('label', null, null, 4)).toBe('Saved label in 4 ms');
  });
  it('shortens a long value so the toast stays readable', () => {
    expect(savedMessage('notes', 'x', 'y'.repeat(100), 4)).toBe(`Saved notes as ${'y'.repeat(40)}…`);
  });
});
