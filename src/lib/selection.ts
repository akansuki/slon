/** Cell selection and text export for the results grid. Pure functions, no DOM. */

export interface Pos {
  r: number;
  c: number;
}

/** Inclusive rectangle in display (sorted) coordinates. */
export interface Range {
  r0: number;
  r1: number;
  c0: number;
  c1: number;
}

export interface Selection {
  anchor: Pos;
  focus: Pos;
  /** Whole rows were selected (row-number click): every column is included. */
  rows: boolean;
}

export function toRange(sel: Selection, colCount: number): Range {
  return {
    r0: Math.min(sel.anchor.r, sel.focus.r),
    r1: Math.max(sel.anchor.r, sel.focus.r),
    c0: sel.rows ? 0 : Math.min(sel.anchor.c, sel.focus.c),
    c1: sel.rows ? colCount - 1 : Math.max(sel.anchor.c, sel.focus.c),
  };
}

export function inRange(range: Range, r: number, c: number): boolean {
  return r >= range.r0 && r <= range.r1 && c >= range.c0 && c <= range.c1;
}

/** Quote a field the way spreadsheets expect when it would break the tab/line layout. */
export function escapeField(v: string): string {
  return /[\t\r\n"]/.test(v) ? `"${v.replaceAll('"', '""')}"` : v;
}

export interface TextSource {
  columns: string[];
  /** Value at display position; `null` is SQL NULL. */
  cell: (r: number, c: number) => string | null;
}

/**
 * Tab-separated text for `range`, ready to paste into a spreadsheet.
 * A single cell without headers is copied verbatim (no quoting). NULL becomes an
 * empty field.
 */
export function toText(src: TextSource, range: Range, headers: boolean): string {
  if (!headers && range.r0 === range.r1 && range.c0 === range.c1) {
    return src.cell(range.r0, range.c0) ?? '';
  }
  const lines: string[] = [];
  if (headers) {
    const h: string[] = [];
    for (let c = range.c0; c <= range.c1; c++) h.push(escapeField(src.columns[c] ?? ''));
    lines.push(h.join('\t'));
  }
  for (let r = range.r0; r <= range.r1; r++) {
    const f: string[] = [];
    for (let c = range.c0; c <= range.c1; c++) f.push(escapeField(src.cell(r, c) ?? ''));
    lines.push(f.join('\t'));
  }
  return lines.join('\n');
}

export function describe(range: Range): string {
  const rows = range.r1 - range.r0 + 1;
  const cols = range.c1 - range.c0 + 1;
  if (rows === 1 && cols === 1) return '1 cell';
  if (cols === 1) return `${rows.toLocaleString()} cells`;
  return `${rows.toLocaleString()} row${rows === 1 ? '' : 's'} × ${cols} column${cols === 1 ? '' : 's'}`;
}

/** Writes to the clipboard; falls back to a hidden textarea where the async API is blocked. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.cssText = 'position:fixed;left:-9999px;top:0;opacity:0';
    document.body.appendChild(ta);
    const active = document.activeElement as HTMLElement | null;
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand('copy');
    } catch {
      ok = false;
    }
    ta.remove();
    active?.focus?.();
    return ok;
  }
}
