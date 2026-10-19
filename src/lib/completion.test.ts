import { describe, expect, it } from 'vitest';
import {
  buildNamespace,
  columnOptions,
  looksLikeDdl,
  quoteIfNeeded,
  resolve,
  sanitize,
  statementAt,
  tablesInStatement,
} from './completion';
import type { CompletionSchema } from './types';

const info: CompletionSchema = {
  defaultSchema: 'public',
  truncated: false,
  relations: [
    {
      schema: 'public',
      name: 'orders',
      kind: 'table',
      columns: [
        { name: 'id', dataType: 'bigint' },
        { name: 'Odd Name', dataType: 'text' },
      ],
    },
    { schema: 'public', name: 'customers', kind: 'table', columns: [{ name: 'email', dataType: 'text' }] },
    { schema: 'audit', name: 'orders', kind: 'view', columns: [{ name: 'changed_at', dataType: 'timestamp' }] },
    { schema: 'Mixed', name: 'Thing', kind: 'materializedView', columns: [] },
  ],
};
const labels = (stmt: string) => columnOptions(info, stmt).map((o) => o.label);

describe('quoteIfNeeded', () => {
  it('quotes only what PostgreSQL would fold or reject', () => {
    expect(quoteIfNeeded('orders')).toBe('orders');
    expect(quoteIfNeeded('order_items2')).toBe('order_items2');
    expect(quoteIfNeeded('Odd Name')).toBe('"Odd Name"');
    expect(quoteIfNeeded('Mixed')).toBe('"Mixed"');
    expect(quoteIfNeeded('1abc')).toBe('"1abc"');
    expect(quoteIfNeeded('a"b')).toBe('"a""b"');
  });
});

describe('sanitize / statementAt', () => {
  it('blanks strings and comments but keeps offsets', () => {
    const s = "select 'from x' -- join y\n, 1 /* from z */";
    const clean = sanitize(s);
    expect(clean.length).toBe(s.length);
    expect(clean).not.toMatch(/from x|join y|from z/);
    expect(clean).toContain('select');
  });
  it('picks the statement around the cursor', () => {
    const t = 'select 1 from a; select 2 from b; select 3 from c';
    expect(statementAt(t, 5)).toContain('from a');
    expect(statementAt(t, 20)).toContain('from b');
    expect(statementAt(t, 20)).not.toContain('from a');
    expect(statementAt(t, t.length)).toContain('from c');
    expect(statementAt(t, 0)).toContain('from a');
  });
  it('a semicolon inside a string does not split', () => {
    const t = "select ';' from a";
    expect(statementAt(t, 3)).toContain('from a');
  });
});

describe('tablesInStatement', () => {
  it('finds from/join with aliases', () => {
    expect(tablesInStatement('select * from orders o join customers as c on o.id = c.id where x')).toEqual([
      { schema: undefined, name: 'orders', alias: 'o' },
      { schema: undefined, name: 'customers', alias: 'c' },
    ]);
  });
  it('does not mistake keywords for aliases', () => {
    const r = tablesInStatement('select * from orders where id = 1');
    expect(r).toEqual([{ schema: undefined, name: 'orders', alias: undefined }]);
    expect(tablesInStatement('select * from orders left join customers on true')[0].alias).toBeUndefined();
  });
  it('handles schema qualification and quoted names', () => {
    expect(tablesInStatement('select * from audit.orders a, "Mixed"."Thing"')).toEqual([
      { schema: 'audit', name: 'orders', alias: 'a' },
    ]);
    expect(tablesInStatement('update "Mixed"."Thing" set x = 1')[0]).toEqual({
      schema: 'Mixed',
      name: 'Thing',
      alias: undefined,
    });
  });
  it('is case-insensitive about keywords and folds unquoted names', () => {
    expect(tablesInStatement('SELECT * FROM Orders JOIN Customers ON TRUE').map((t) => t.name)).toEqual([
      'orders',
      'customers',
    ]);
  });
});

describe('resolve', () => {
  it('prefers the default schema, then any, honours explicit schema', () => {
    expect(resolve(info, { name: 'orders' })?.schema).toBe('public');
    expect(resolve(info, { schema: 'audit', name: 'orders' })?.kind).toBe('view');
    expect(resolve(info, { name: 'thing' })).toBeUndefined();
    expect(resolve(info, { schema: 'Mixed', name: 'Thing' })?.kind).toBe('materializedView');
    expect(resolve(info, { name: 'nope' })).toBeUndefined();
  });
});

describe('columnOptions', () => {
  it('offers the columns of the tables in the statement only', () => {
    expect(labels('select  from orders')).toEqual(['id', 'Odd Name']);
    expect(labels('select  from orders join customers on true')).toEqual(['id', 'Odd Name', 'email']);
    expect(labels('select 1')).toEqual([]);
  });
  it('quotes odd column names when applied and shows table and type', () => {
    const odd = columnOptions(info, 'from orders').find((o) => o.label === 'Odd Name')!;
    expect(odd.apply).toBe('"Odd Name"');
    expect(odd.detail).toBe('orders · text');
  });
  it('same table twice (self join) is offered once', () => {
    expect(labels('from orders a join orders b on true')).toEqual(['id', 'Odd Name']);
  });
});

describe('buildNamespace', () => {
  it('nests schema > relation > columns and exposes the default schema', () => {
    const ns = buildNamespace(info);
    expect(ns.defaultSchema).toBe('public');
    const pub = (ns.schema as any).public;
    expect(Object.keys(pub.children)).toEqual(['orders', 'customers']);
    expect(pub.children.orders.children.map((c: any) => c.label)).toEqual(['id', 'Odd Name']);
    expect(pub.children.orders.self.detail).toBe('table');
    expect((ns.schema as any).Mixed.self.apply).toBe('"Mixed"');
    expect((ns.schema as any).Mixed.children.Thing.self.apply).toBe('"Thing"');
  });
  it('null default schema becomes undefined', () => {
    expect(buildNamespace({ ...info, defaultSchema: null }).defaultSchema).toBeUndefined();
  });
});

describe('looksLikeDdl', () => {
  it('detects schema changes, ignores strings and comments', () => {
    expect(looksLikeDdl('CREATE TABLE x(id int)')).toBe(true);
    expect(looksLikeDdl('select 1; drop table x')).toBe(true);
    expect(looksLikeDdl("select 'create table'")).toBe(false);
    expect(looksLikeDdl('select 1 -- drop it')).toBe(false);
    expect(looksLikeDdl('select * from t')).toBe(false);
  });
});
