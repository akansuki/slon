import { describe, expect, it } from 'vitest';
import {
  DEFAULT_OPTIONS,
  extensionFor,
  labelFor,
  sanitizeOptions,
  suggestName,
  type ExportOptions,
} from './export';

const opts = (over: Partial<ExportOptions> = {}): ExportOptions => ({ ...DEFAULT_OPTIONS, ...over });

// The file format itself is tested in Rust (crates/pg-core/src/export.rs), which is where
// the formatting lives: both export paths stream too much data to pass through the page.

describe('file names', () => {
  it('picks the extension from the options', () => {
    expect(extensionFor(opts())).toBe('csv');
    expect(extensionFor(opts({ delimiter: ';' }))).toBe('csv');
    expect(extensionFor(opts({ delimiter: '\t' }))).toBe('tsv');
    expect(extensionFor(opts({ format: 'json' }))).toBe('json');
    expect(extensionFor(opts({ format: 'jsonl', delimiter: '\t' }))).toBe('jsonl');
  });

  it('suggests a time-stamped name', () => {
    expect(suggestName(opts(), new Date(2026, 9, 4, 8, 5, 9))).toBe('result-20261004-080509.csv');
  });

  it('names the format for the save dialog filter', () => {
    expect(labelFor(opts())).toBe('CSV');
    expect(labelFor(opts({ delimiter: '\t' }))).toBe('Tab-separated values');
    expect(labelFor(opts({ format: 'json' }))).toBe('JSON');
    expect(labelFor(opts({ format: 'jsonl' }))).toBe('JSON Lines');
  });
});

describe('remembered options', () => {
  it('keeps what is valid', () => {
    const stored = { format: 'jsonl', delimiter: ';', header: false, bom: false };
    expect(sanitizeOptions(stored)).toEqual(stored);
  });

  it('falls back to the defaults for anything else', () => {
    expect(sanitizeOptions(null)).toEqual(DEFAULT_OPTIONS);
    expect(sanitizeOptions('csv')).toEqual(DEFAULT_OPTIONS);
    expect(sanitizeOptions({ format: 'xlsx', delimiter: '|', header: 'yes' })).toEqual(DEFAULT_OPTIONS);
    // A half-valid object keeps the parts that make sense.
    expect(sanitizeOptions({ format: 'json', delimiter: '|' })).toEqual({
      ...DEFAULT_OPTIONS,
      format: 'json',
    });
  });
});
