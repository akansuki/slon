import { describe, expect, it } from 'vitest';
import { clampSidebar, parseStoredSidebar, sidebarMax, SIDEBAR_DEFAULT, SIDEBAR_MIN } from './layout';

describe('sidebar width', () => {
  it('stays between the minimum and what the window leaves over', () => {
    expect(clampSidebar(50, 1600)).toBe(SIDEBAR_MIN);
    expect(clampSidebar(400, 1600)).toBe(400);
    expect(clampSidebar(5000, 1600)).toBe(800);
    expect(clampSidebar(900, 900)).toBe(540); // leaves 360 for the query area
    expect(clampSidebar(300, 300)).toBe(SIDEBAR_MIN); // a tiny window never pushes it below the minimum
    expect(sidebarMax(500)).toBe(SIDEBAR_MIN);
  });

  it('rounds to whole pixels and survives nonsense', () => {
    expect(clampSidebar(300.6, 1600)).toBe(301);
    expect(clampSidebar(NaN, 1600)).toBe(SIDEBAR_DEFAULT);
  });

  it('only trusts sensible stored values', () => {
    expect(parseStoredSidebar('320')).toBe(320);
    expect(parseStoredSidebar(null)).toBeNull();
    expect(parseStoredSidebar('')).toBeNull();
    expect(parseStoredSidebar('abc')).toBeNull();
    expect(parseStoredSidebar('10')).toBeNull();
    expect(parseStoredSidebar('99999')).toBeNull();
  });
});
