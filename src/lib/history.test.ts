import { describe, expect, it } from 'vitest';
import { durationLabel, preview, whenLabel } from './history';

describe('preview', () => {
  it('keeps short statements as they are', () => {
    expect(preview('select 1')).toBe('select 1');
    expect(preview('  select 1\n')).toBe('select 1');
  });
  it('cuts long statements by lines and by characters', () => {
    expect(preview('a\nb\nc\nd\ne', 3)).toBe('a\nb\nc …');
    expect(preview('x'.repeat(500), 4, 10)).toBe('xxxxxxxxxx …');
  });
});

describe('whenLabel', () => {
  const now = new Date(2026, 9, 14, 15, 30).getTime();
  it('is relative for the last hour', () => {
    expect(whenLabel(now - 10_000, now)).toBe('just now');
    expect(whenLabel(now - 5 * 60_000, now)).toBe('5 min ago');
  });
  it('shows the clock time today, the date earlier', () => {
    expect(whenLabel(new Date(2026, 9, 14, 9, 5).getTime(), now)).toBe('09:05');
    expect(whenLabel(new Date(2026, 9, 12, 9, 5).getTime(), now)).toBe('12 Oct 09:05');
    expect(whenLabel(new Date(2025, 11, 31, 9, 5).getTime(), now)).toBe('31 Dec 2025');
  });
});

describe('durationLabel', () => {
  it('uses ms below a second', () => {
    expect(durationLabel(12)).toBe('12 ms');
    expect(durationLabel(1500)).toBe('1.50 s');
  });
});
