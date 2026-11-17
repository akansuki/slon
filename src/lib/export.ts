/**
 * Export options, and the names that go with them.
 *
 * The formatting itself lives in Rust (`crates/pg-core/src/export.rs`). It used to be here,
 * but "Export all rows" streams results that are far too big to pass through the page, so
 * there would have been two implementations of the same file format — and two that must
 * agree byte for byte is one too many. The page now decides *what* to save and the backend
 * decides what the bytes are.
 */
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

/** Rows sent to the backend per call, when the grid's rows are the source. */
export const CHUNK_ROWS = 5000;

export function extensionFor(o: ExportOptions): 'csv' | 'tsv' | 'json' | 'jsonl' {
  if (o.format === 'json') return 'json';
  if (o.format === 'jsonl') return 'jsonl';
  return o.delimiter === '\t' ? 'tsv' : 'csv';
}

/** What to call the format in the save dialog's filter. */
export function labelFor(o: ExportOptions): string {
  if (o.format === 'json') return 'JSON';
  if (o.format === 'jsonl') return 'JSON Lines';
  return o.delimiter === '\t' ? 'Tab-separated values' : 'CSV';
}

function pad(n: number) {
  return String(n).padStart(2, '0');
}

export function suggestName(o: ExportOptions, now = new Date()): string {
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
  return `result-${stamp}.${extensionFor(o)}`;
}

/** Read options back from storage without trusting what is there. */
export function sanitizeOptions(raw: unknown): ExportOptions {
  if (!raw || typeof raw !== 'object') return { ...DEFAULT_OPTIONS };
  const o = raw as Record<string, unknown>;
  return {
    format: (['csv', 'json', 'jsonl'] as const).find((f) => f === o.format) ?? DEFAULT_OPTIONS.format,
    delimiter: ([',', ';', '\t'] as const).find((d) => d === o.delimiter) ?? DEFAULT_OPTIONS.delimiter,
    header: typeof o.header === 'boolean' ? o.header : DEFAULT_OPTIONS.header,
    bom: typeof o.bom === 'boolean' ? o.bom : DEFAULT_OPTIONS.bom,
  };
}

const KEY = 'slon.exportOptions';

export function loadOptions(): ExportOptions {
  try {
    return sanitizeOptions(JSON.parse(localStorage.getItem(KEY) ?? 'null'));
  } catch {
    return { ...DEFAULT_OPTIONS };
  }
}

export function rememberOptions(o: ExportOptions) {
  try {
    localStorage.setItem(KEY, JSON.stringify(o));
  } catch {
    /* not remembered */
  }
}
