/** Turns the database's table/column names into CodeMirror autocompletion. */
import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
import type { SQLNamespace } from '@codemirror/lang-sql';
import type { CompletionRelation, CompletionSchema, RelationKind } from './types';

/** Quote an identifier only when PostgreSQL would otherwise change or reject it. */
export function quoteIfNeeded(name: string): string {
  return /^[a-z_][a-z0-9_$]*$/.test(name) ? name : `"${name.replaceAll('"', '""')}"`;
}

const KIND_LABEL: Record<RelationKind, string> = {
  table: 'table',
  view: 'view',
  materializedView: 'materialized view',
  foreignTable: 'foreign table',
};

const KIND_TYPE: Record<RelationKind, string> = {
  table: 'class',
  view: 'interface',
  materializedView: 'interface',
  foreignTable: 'class',
};

function columnCompletion(r: CompletionRelation, i: number): Completion {
  const c = r.columns[i];
  return { label: c.name, apply: quoteIfNeeded(c.name), type: 'property', detail: c.dataType, boost: -i / 1000 };
}

/** Namespace for `sql({ schema, defaultSchema })`: schema -> relation -> columns. */
export function buildNamespace(info: CompletionSchema): {
  schema: SQLNamespace;
  defaultSchema: string | undefined;
} {
  const schemas = new Map<string, Record<string, SQLNamespace>>();
  for (const r of info.relations) {
    let tables = schemas.get(r.schema);
    if (!tables) schemas.set(r.schema, (tables = {}));
    tables[r.name] = {
      self: { label: r.name, apply: quoteIfNeeded(r.name), type: KIND_TYPE[r.kind], detail: KIND_LABEL[r.kind] },
      children: r.columns.map((_, i) => columnCompletion(r, i)),
    };
  }
  const schema: Record<string, SQLNamespace> = {};
  for (const [name, tables] of schemas) {
    schema[name] = {
      self: { label: name, apply: quoteIfNeeded(name), type: 'namespace', detail: 'schema' },
      children: tables,
    };
  }
  return { schema, defaultSchema: info.defaultSchema ?? undefined };
}

/* ------------------------------------------------------- columns of the current statement */

export interface TableRef {
  schema?: string;
  name: string;
  alias?: string;
}

/** Blank out comments and string literals, keeping every offset intact. */
export function sanitize(text: string): string {
  const blank = (m: string) => m.replace(/[^\n]/g, ' ');
  return text
    .replace(/\/\*[\s\S]*?(\*\/|$)/g, blank)
    .replace(/--[^\n]*/g, blank)
    .replace(/'(?:[^']|'')*'?/g, blank);
}

/** The `;`-delimited statement containing `pos`, as clean text. */
export function statementAt(text: string, pos: number): string {
  const clean = sanitize(text);
  const from = pos === 0 ? 0 : clean.lastIndexOf(';', pos - 1) + 1;
  const end = clean.indexOf(';', pos);
  return clean.slice(from, end === -1 ? undefined : end);
}

const ID = String.raw`(?:"(?:[^"]|"")+"|[A-Za-z_][\w$]*)`;
// Words that can follow a table name without being its alias.
const NOT_ALIAS = (
  'where on using join inner left right full cross natural outer group order limit offset having union ' +
  'intersect except set values select returning window for fetch lateral tablesample and or'
).replaceAll(' ', '|');
// The alias group must not consume a keyword, or `from a join b` would swallow `join`.
const REF = new RegExp(
  String.raw`\b(?:from|join|update|into|table)\s+(${ID}(?:\s*\.\s*${ID})?)` +
    String.raw`(?:\s+(?:as\s+)?(?!(?:${NOT_ALIAS})\b)(${ID}))?`,
  'gi',
);

const unquote = (s: string) => (s.startsWith('"') ? s.slice(1, -1).replaceAll('""', '"') : s.toLowerCase());

/** Tables a statement refers to (FROM / JOIN / UPDATE / INTO), with their aliases. */
export function tablesInStatement(stmt: string): TableRef[] {
  const refs: TableRef[] = [];
  for (const m of stmt.matchAll(REF)) {
    const parts = m[1].split(/\s*\.\s*/);
    const name = unquote(parts[parts.length - 1]);
    const schema = parts.length > 1 ? unquote(parts[0]) : undefined;
    refs.push({ schema, name, alias: m[2] ? unquote(m[2]) : undefined });
  }
  return refs;
}

/** Resolve a reference against the known relations (search-path schema first). */
export function resolve(info: CompletionSchema, ref: TableRef): CompletionRelation | undefined {
  const same = (r: CompletionRelation) => r.name === ref.name;
  if (ref.schema) return info.relations.find((r) => r.schema === ref.schema && same(r));
  return (
    info.relations.find((r) => r.schema === info.defaultSchema && same(r)) ??
    info.relations.find(same)
  );
}

/** Column suggestions for the tables a statement uses. */
export function columnOptions(info: CompletionSchema, stmt: string): Completion[] {
  const seen = new Set<CompletionRelation>();
  const out: Completion[] = [];
  for (const ref of tablesInStatement(stmt)) {
    const rel = resolve(info, ref);
    if (!rel || seen.has(rel)) continue;
    seen.add(rel);
    rel.columns.forEach((c, i) => {
      out.push({
        label: c.name,
        apply: quoteIfNeeded(c.name),
        type: 'property',
        detail: `${rel.name} · ${c.dataType}`,
        boost: 5 - i / 1000, // above keywords: inside a query the columns are usually what you want
      });
    });
  }
  return out;
}

/**
 * CodeMirror completion source for bare column names. Dotted names (`t.col`,
 * `schema.table`) are handled by the SQL language's own schema completion.
 */
export function columnCompletionSource(getInfo: () => CompletionSchema | null) {
  return (ctx: CompletionContext): CompletionResult | null => {
    const info = getInfo();
    if (!info) return null;
    const word = ctx.matchBefore(/[\w$]*/);
    if (!word || (word.from === word.to && !ctx.explicit)) return null;
    const prev = ctx.state.sliceDoc(Math.max(0, word.from - 1), word.from);
    if (prev === '.' || prev === '"') return null;
    const stmt = statementAt(ctx.state.doc.toString(), ctx.pos);
    const options = columnOptions(info, stmt);
    if (options.length === 0) return null;
    return { from: word.from, options, validFor: /^[\w$]*$/ };
  };
}

/** Does this SQL change which tables/columns exist? Used to refresh the completion data. */
export function looksLikeDdl(sql: string): boolean {
  return /\b(create|alter|drop|rename|truncate)\b/i.test(sanitize(sql));
}
