/**
 * Working out whether the rows selected in the grid can be deleted, and which key values
 * identify them. Pure functions, no DOM and no API calls.
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

export interface Deletability {
    ok: boolean;
    /** Why not, phrased for a tooltip. Empty when deletable. */
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
): Deletability {
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
