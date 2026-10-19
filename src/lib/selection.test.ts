import { describe as suite, expect, it } from 'vitest';
import { describe, escapeField, inRange, toRange, toText, type Selection } from './selection';

const data: (string | null)[][] = [
  ['1', 'alpha', null],
  ['2', 'be\tta', 'say "hi"'],
  ['3', 'line\nbreak', ''],
];
const src = { columns: ['id', 'name', 'note'], cell: (r: number, c: number) => data[r][c] };

suite('toRange', () => {
  it('normalizes a backwards drag', () => {
    const s: Selection = { anchor: { r: 5, c: 3 }, focus: { r: 2, c: 1 }, rows: false };
    expect(toRange(s, 4)).toEqual({ r0: 2, r1: 5, c0: 1, c1: 3 });
  });
  it('row selection spans every column', () => {
    const s: Selection = { anchor: { r: 1, c: 2 }, focus: { r: 3, c: 0 }, rows: true };
    expect(toRange(s, 4)).toEqual({ r0: 1, r1: 3, c0: 0, c1: 3 });
  });
  it('inRange is inclusive', () => {
    const r = { r0: 1, r1: 2, c0: 1, c1: 2 };
    expect(inRange(r, 1, 1) && inRange(r, 2, 2)).toBe(true);
    expect(inRange(r, 0, 1) || inRange(r, 3, 1) || inRange(r, 1, 0) || inRange(r, 1, 3)).toBe(false);
  });
});

suite('escapeField', () => {
  it('leaves plain text alone', () => expect(escapeField('abc 123')).toBe('abc 123'));
  it('quotes tabs, newlines and quotes', () => {
    expect(escapeField('a\tb')).toBe('"a\tb"');
    expect(escapeField('a\nb')).toBe('"a\nb"');
    expect(escapeField('a\r\nb')).toBe('"a\r\nb"');
    expect(escapeField('say "hi"')).toBe('"say ""hi"""');
  });
});

suite('toText', () => {
  it('single cell is verbatim, even with special characters', () => {
    expect(toText(src, { r0: 1, r1: 1, c0: 2, c1: 2 }, false)).toBe('say "hi"');
    expect(toText(src, { r0: 2, r1: 2, c0: 1, c1: 1 }, false)).toBe('line\nbreak');
  });
  it('single NULL cell is empty', () => {
    expect(toText(src, { r0: 0, r1: 0, c0: 2, c1: 2 }, false)).toBe('');
  });
  it('rectangle is tab separated, NULL empty, specials quoted', () => {
    expect(toText(src, { r0: 0, r1: 1, c0: 0, c1: 2 }, false)).toBe('1\talpha\t\n2\t"be\tta"\t"say ""hi"""');
  });
  it('headers are included and escaped', () => {
    expect(toText(src, { r0: 0, r1: 0, c0: 0, c1: 1 }, true)).toBe('id\tname\n1\talpha');
    expect(toText(src, { r0: 0, r1: 0, c0: 2, c1: 2 }, true)).toBe('note\n');
  });
});

suite('describe', () => {
  it('words the size of the selection', () => {
    expect(describe({ r0: 0, r1: 0, c0: 0, c1: 0 })).toBe('1 cell');
    expect(describe({ r0: 0, r1: 4, c0: 2, c1: 2 })).toBe('5 cells');
    expect(describe({ r0: 0, r1: 0, c0: 0, c1: 3 })).toBe('1 row × 4 columns');
    expect(describe({ r0: 0, r1: 1, c0: 0, c1: 1 })).toBe('2 rows × 2 columns');
  });
});
