/**
 * Turning a result set into CSV / JSON text. Pure functions: the dialog feeds them chunks of
 * rows and hands each piece of text to the backend, which appends it to the chosen file, so
 * a 100,000-row result never has to exist as one huge string.
 */
import type { Cell, Column } from './types';

export type ExportFormat = 'csv' | 'json' | 'jsonl';
export type Delimiter = ',' | ';' | '\t';

export interface ExportOptions {
  format: ExportFormat;
  /** CSV only. */
  delimiter: Delimiter;
  /** CSV only: first line holds the column names. */
  header: boolean;
  /** CSV only: UTF-8 byte order mark. Excel on Windows needs it to read accented letters. */
  bom: boolean;
}

export const DEFAULT_OPTIONS: ExportOptions = { format: 'csv', delimiter: ',', header: true, bom: true };

/** Rows handed to the formatter per call. */
export const CHUNK_ROWS = 5000;

export function extensionFor(o: ExportOptions): 'csv' | 'tsv' | 'json' | 'jsonl' {
  if (o.format === 'json') return 'json';
  if (o.format === 'jsonl') return 'jsonl';
  return o.delimiter === '\t' ? 'tsv' : 'csv';
}

function pad(n: number) {
  return String(n).padStart(2, '0');
}

export function suggestName(o: ExportOptions, now = new Date()): string {
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
  return `result-${stamp}.${extensionFor(o)}`;
}

/* ------------------------------------------------------------------ CSV */

/**
 * RFC 4180 quoting. Like PostgreSQL's own `COPY ... CSV`, NULL is an empty field and an empty
 * string is written as `""`, so the two stay apart when the file is loaded back.
 */
export function csvField(value: Cell, delimiter: string): string {
  if (value === null) return '';
  if (value === '') return '""';
  return value.includes(delimiter) || /["\r\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value;
}

function csvLine(cells: readonly Cell[], delimiter: string): string {
  return cells.map((c) => csvField(c, delimiter)).join(delimiter) + '\r\n';
}

/* ------------------------------------------------------------------ JSON */

const NUMERIC_TYPES = new Set(['int2', 'int4', 'int8', 'float4', 'float8', 'numeric', 'oid']);
// The JSON grammar for numbers. PostgreSQL prints NaN and Infinity as words, which JSON cannot hold.
const JSON_NUMBER = /^-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?$/;

/**
 * One value as JSON text. Numbers keep the digits the server printed (a bigint or numeric
 * would lose precision as a JavaScript number), booleans become true/false, json and jsonb
 * values are embedded as they are, everything else is a string.
 */
export function jsonValue(cell: Cell, column: Column): string {
  if (cell === null) return 'null';
  const type = column.typeName;
  if (type && NUMERIC_TYPES.has(type) && JSON_NUMBER.test(cell)) return cell;
  if (type === 'bool') {
    if (cell === 't') return 'true';
    if (cell === 'f') return 'false';
  }
  if (type === 'json' || type === 'jsonb') {
    try {
      JSON.parse(cell);
      return cell;
    } catch {
      /* not valid after all: fall through and keep it as text */
    }
  }
  return JSON.stringify(cell);
}

/** Column names as object keys; a repeated name gets `_2`, `_3`, ... so no value is lost. */
export function jsonKeys(columns: readonly Column[]): string[] {
  const seen = new Set<string>();
  return columns.map((c) => {
    let key = c.name;
    for (let n = 2; seen.has(key); n++) key = `${c.name}_${n}`;
    seen.add(key);
    return JSON.stringify(key);
  });
}

function jsonObject(row: readonly Cell[], columns: readonly Column[], keys: readonly string[]): string {
  return `{${columns.map((c, i) => `${keys[i]}: ${jsonValue(row[i] ?? null, c)}`).join(', ')}}`;
}

/* ------------------------------------------------------------------ pieces of a file */

/** Text that opens the file. */
export function exportHeader(columns: readonly Column[], o: ExportOptions): string {
  switch (o.format) {
    case 'csv':
      return (o.bom ? '﻿' : '') + (o.header ? csvLine(columns.map((c) => c.name), o.delimiter) : '');
    case 'json':
      return '[';
    case 'jsonl':
      return '';
  }
}

/** Text for rows `from` (inclusive) to `to` (exclusive). `from` is the row's index in the whole result. */
export function exportRows(
  columns: readonly Column[],
  rows: readonly (readonly Cell[])[],
  from: number,
  to: number,
  o: ExportOptions,
): string {
  const end = Math.min(to, rows.length);
  const parts: string[] = [];
  const keys = o.format === 'csv' ? [] : jsonKeys(columns);
  for (let i = from; i < end; i++) {
    switch (o.format) {
      case 'csv':
        parts.push(csvLine(rows[i], o.delimiter));
        break;
      case 'json':
        parts.push((i === 0 ? '\n  ' : ',\n  ') + jsonObject(rows[i], columns, keys));
        break;
      case 'jsonl':
        parts.push(jsonObject(rows[i], columns, keys) + '\n');
        break;
    }
  }
  return parts.join('');
}

/** Text that closes the file. */
export function exportFooter(rowCount: number, o: ExportOptions): string {
  return o.format === 'json' ? (rowCount > 0 ? '\n]\n' : ']\n') : '';
}

/** The whole file as one string. Only for small results and tests; the dialog streams chunks. */
export function exportAll(columns: readonly Column[], rows: readonly (readonly Cell[])[], o: ExportOptions): string {
  return exportHeader(columns, o) + exportRows(columns, rows, 0, rows.length, o) + exportFooter(rows.length, o);
}
