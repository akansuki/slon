/** Sidebar width: limits and what is remembered between runs. Pure, so it can be tested. */

export const SIDEBAR_DEFAULT = 290;
export const SIDEBAR_MIN = 180;
const SIDEBAR_MAX = 800;
/** The query area must keep at least this much room. */
const MAIN_MIN = 360;
export const SIDEBAR_KEY = 'slon.sidebarWidth';

/** Largest width that still leaves room for the query area in a window `windowWidth` wide. */
export function sidebarMax(windowWidth: number): number {
  return Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, windowWidth - MAIN_MIN));
}

export function clampSidebar(width: number, windowWidth: number): number {
  if (!Number.isFinite(width)) return SIDEBAR_DEFAULT;
  return Math.round(Math.min(Math.max(width, SIDEBAR_MIN), sidebarMax(windowWidth)));
}

/** A stored value that is not a sensible number is ignored. */
export function parseStoredSidebar(raw: string | null): number | null {
  if (raw === null || raw.trim() === '') return null;
  const n = Number(raw);
  return Number.isFinite(n) && n >= SIDEBAR_MIN && n <= SIDEBAR_MAX ? n : null;
}
