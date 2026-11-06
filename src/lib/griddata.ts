/**
 * Working out what the grid is allowed to change: which rows can be deleted, which cells
 * can be edited, and the key values that identify a row. Pure functions, no DOM and no
 * API calls.
 *
 * The server tells us, per result column, which table it came from (`tableOid`) and its
 * position in that table (`columnId`). A row is only deletable when every column came from
 * one ordinary table and that table's whole primary key is present in the result. Matching
 * the key by position rather than by name means `SELECT id AS whatever` still works.
 */
import type { Cell, Column, RowIdentity } from './types';

/** The table every column came from, or null when they disagree or any is computed. */
export function sourceTableOid(columns: Column[]): number | null {
    if (columns.length === 0) return null;
    const first = columns[0].tableOid;
    if (first === null || first === undefined) return null;
    return columns.every((c) => c.tableOid === first) ? first : null;
}

/**
 * Where each key column sits in the result, in key order. Null when the query left one out
 * (`SELECT label FROM items` has no id to delete by).
 */
export function keyIndexes(columns: Column[], identity: RowIdentity): number[] | null {
    const out: number[] = [];
    for (const k of identity.key) {
        const i = columns.findIndex((c) => c.columnId === k.attnum);
        if (i < 0) return null;
        out.push(i);
    }
    return out;
}

/** Whether an action is allowed, and what to say when it is not. */
export interface Verdict {
    ok: boolean;
    /** Phrased for a tooltip. Empty when allowed. */
    reason: string;
}

/**
 * Whether the Delete action should be offered, and what to say when it should not. Every
 * refusal names the thing to change, because "nothing happens" is the worst answer.
 */
export function deletability(
    columns: Column[],
    identity: RowIdentity | null,
    identityError: string | null,
    selectedRows: number[],
    running: boolean,
): Verdict {
    if (running) return { ok: false, reason: 'Wait for the query to finish' };
    if (columns.length === 0) return { ok: false, reason: 'No results to delete from' };
    if (sourceTableOid(columns) === null) {
        return {
            ok: false,
            reason: 'These rows do not come from a single table, so Slon cannot tell which row to delete',
        };
    }
    if (identityError) return { ok: false, reason: identityError };
    if (!identity) return { ok: false, reason: 'Still working out where these rows come from' };
    if (keyIndexes(columns, identity) === null) {
        const missing = identity.key.map((k) => k.name).join(', ');
        return {
            ok: false,
            reason: `Add the primary key (${missing}) to the query to delete from the grid`,
        };
    }
    if (selectedRows.length === 0) {
        return { ok: false, reason: 'Select whole rows first: click a row number' };
    }
    return { ok: true, reason: '' };
}

/**
 * The key values of the selected rows, in key order, ready for the backend.
 *
 * Returns null if any key value is NULL, which should be impossible for a primary key but
 * would otherwise silently turn into a statement that matches nothing.
 */
export function keyValues(
    rows: Cell[][],
    dataRows: number[],
    indexes: number[],
): string[][] | null {
    const out: string[][] = [];
    for (const r of dataRows) {
        const row = rows[r];
        if (!row) return null;
        const values: string[] = [];
        for (const i of indexes) {
            const v = row[i];
            if (v === null || v === undefined) return null;
            values.push(v);
        }
        out.push(values);
    }
    return out;
}

/** "1 row" / "3 rows", used in the button and the confirmation. */
export function rowsLabel(n: number): string {
    return `${n.toLocaleString()} row${n === 1 ? '' : 's'}`;
}

/**
 * Whether each result column can be written, in column order. A column is editable when it
 * is a real column of the source table and the server would accept an UPDATE of it, so a
 * computed column, a generated column and a GENERATED ALWAYS identity are all excluded.
 *
 * The whole result is read-only unless the row can be identified at all, because an edit
 * needs the same key a delete does.
 */
export function editableColumns(columns: Column[], identity: RowIdentity | null): Verdict[] {
    const none = (reason: string) => columns.map(() => ({ ok: false, reason }));
    if (!identity) return none('These rows do not come from a table that can be edited');
    if (keyIndexes(columns, identity) === null) {
        const missing = identity.key.map((k) => k.name).join(', ');
        return none(`Add the primary key (${missing}) to the query to edit these rows`);
    }
    return columns.map((c) => {
        const source = c.columnId === null ? undefined : identity.columns.find((t) => t.attnum === c.columnId);
        if (!source) return { ok: false, reason: `${c.name} is computed, not a stored column` };
        if (!source.editable) return { ok: false, reason: `${source.name} is ${source.reason}` };
        return { ok: true, reason: '' };
    });
}

/**
 * What to say after a cell is written. When the server stored something other than what
 * was typed — `1.5` into a `numeric(10,2)` becomes `1.50` — the message says so, because
 * otherwise the value visibly changing looks like the edit went wrong.
 */
export function savedMessage(
    column: string,
    typed: string | null,
    stored: string | null,
    elapsedMs: number,
): string {
    const show = (v: string | null) =>
        v === null ? 'NULL' : v.length > 40 ? `${v.slice(0, 40)}…` : v;
    return stored === typed
        ? `Saved ${column} in ${elapsedMs} ms`
        : `Saved ${column} as ${show(stored)}`;
}
