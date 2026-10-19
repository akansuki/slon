import { describe, expect, it } from 'vitest';
import {
  csvField,
  DEFAULT_OPTIONS,
  exportAll,
  exportFooter,
  exportHeader,
  exportRows,
  extensionFor,
  jsonKeys,
  jsonValue,
  suggestName,
  type ExportOptions,
} from './export';
import type { Column } from './types';

const col = (name: string, typeName: string | null = null): Column => ({
  name,
  typeName,
  sortKind: typeName === 'int4' ? 'number' : 'text',
});
const opts = (over: Partial<ExportOptions> = {}): ExportOptions => ({ ...DEFAULT_OPTIONS, ...over });

describe('csv', () => {
  it('quotes only what needs it', () => {
    expect(csvField('plain', ',')).toBe('plain');
    expect(csvField('a,b', ',')).toBe('"a,b"');
    expect(csvField('a;b', ',')).toBe('a;b');
    expect(csvField('a;b', ';')).toBe('"a;b"');
    expect(csvField('say "hi"', ',')).toBe('"say ""hi"""');
    expect(csvField('two\nlines', ',')).toBe('"two\nlines"');
    expect(csvField('cr\rhere', ',')).toBe('"cr\rhere"');
    expect(csvField('tab\there', '\t')).toBe('"tab\there"');
    expect(csvField(' padded ', ',')).toBe(' padded ');
  });

  it('keeps NULL and the empty string apart, like COPY ... CSV', () => {
    expect(csvField(null, ',')).toBe('');
    expect(csvField('', ',')).toBe('""');
  });

  it('writes a header, CRLF line ends and the byte order mark', () => {
    const cols = [col('id'), col('name')];
    const rows = [
      ['1', 'Ana'],
      ['2', null],
    ];
    expect(exportAll(cols, rows, opts({ bom: false }))).toBe('id,name\r\n1,Ana\r\n2,\r\n');
    expect(exportAll(cols, rows, opts())).toBe('﻿id,name\r\n1,Ana\r\n2,\r\n');
    expect(exportAll(cols, rows, opts({ header: false, bom: false }))).toBe('1,Ana\r\n2,\r\n');
    expect(exportAll(cols, rows, opts({ delimiter: ';', bom: false }))).toBe('id;name\r\n1;Ana\r\n2;\r\n');
    expect(exportAll(cols, rows, opts({ delimiter: '\t', bom: false }))).toBe('id\tname\r\n1\tAna\r\n2\t\r\n');
  });

  it('quotes column names too', () => {
    expect(exportHeader([col('a,b'), col('c')], opts({ bom: false }))).toBe('"a,b",c\r\n');
  });

  it('header only for an empty result', () => {
    expect(exportAll([col('x')], [], opts({ bom: false }))).toBe('x\r\n');
  });
});

describe('json', () => {
  it('keeps numbers exact, maps booleans, embeds json, quotes the rest', () => {
    expect(jsonValue('42', col('a', 'int4'))).toBe('42');
    expect(jsonValue('-1.50', col('a', 'numeric'))).toBe('-1.50');
    expect(jsonValue('9223372036854775807', col('a', 'int8'))).toBe('9223372036854775807');
    expect(jsonValue('1e+20', col('a', 'float8'))).toBe('1e+20');
    expect(jsonValue('NaN', col('a', 'float8'))).toBe('"NaN"');
    expect(jsonValue('Infinity', col('a', 'float8'))).toBe('"Infinity"');
    expect(jsonValue('t', col('a', 'bool'))).toBe('true');
    expect(jsonValue('f', col('a', 'bool'))).toBe('false');
    expect(jsonValue('{"a": [1, 2]}', col('a', 'jsonb'))).toBe('{"a": [1, 2]}');
    expect(jsonValue('not json', col('a', 'json'))).toBe('"not json"');
    expect(jsonValue('2024-01-02', col('a', 'date'))).toBe('"2024-01-02"');
    expect(jsonValue('say "hi"\n', col('a', 'text'))).toBe('"say \\"hi\\"\\n"');
    expect(jsonValue(null, col('a', 'int4'))).toBe('null');
  });

  it('does not guess types when they are unknown (scripts)', () => {
    expect(jsonValue('42', col('a', null))).toBe('"42"');
    expect(jsonValue('t', col('a', null))).toBe('"t"');
  });

  it('writes an array with one object per line', () => {
    const cols = [col('id', 'int4'), col('name', 'text')];
    const rows = [
      ['1', 'Ana'],
      ['2', null],
    ];
    expect(exportAll(cols, rows, opts({ format: 'json' }))).toBe(
      '[\n  {"id": 1, "name": "Ana"},\n  {"id": 2, "name": null}\n]\n',
    );
    expect(exportAll(cols, [], opts({ format: 'json' }))).toBe('[]\n');
  });

  it('jsonl is one object per line without brackets', () => {
    expect(exportAll([col('id', 'int4')], [['1'], ['2']], opts({ format: 'jsonl' }))).toBe('{"id": 1}\n{"id": 2}\n');
  });

  it('keeps repeated column names apart', () => {
    expect(jsonKeys([col('a'), col('a'), col('b'), col('a')])).toEqual(['"a"', '"a_2"', '"b"', '"a_3"']);
    expect(exportAll([col('a', 'int4'), col('a', 'int4')], [['1', '2']], opts({ format: 'jsonl' }))).toBe('{"a": 1, "a_2": 2}\n');
  });

  it('is valid JSON for awkward values', () => {
    const cols = [col('t', 'text'), col('n', 'numeric'), col('j', 'jsonb')];
    const rows = [['quote " backslash \\ newline \n tab \t unicode č ć š đ ž 😀', '0.10', '{"x": null}']];
    const text = exportAll(cols, rows, opts({ format: 'json' }));
    expect(JSON.parse(text)).toEqual([{ t: rows[0][0], n: 0.1, j: { x: null } }]);
  });
});

describe('chunks add up to the whole file', () => {
  it.each(['csv', 'json', 'jsonl'] as const)('%s', (format) => {
    const cols = [col('id', 'int4'), col('v', 'text')];
    const rows = Array.from({ length: 11 }, (_, i) => [String(i), `row ${i}, "x"`]);
    const o = opts({ format, bom: false });
    let pieces = exportHeader(cols, o);
    for (let from = 0; from < rows.length; from += 4) pieces += exportRows(cols, rows, from, from + 4, o);
    pieces += exportFooter(rows.length, o);
    expect(pieces).toBe(exportAll(cols, rows, o));
    if (format === 'json') expect(JSON.parse(pieces)).toHaveLength(11);
  });
});

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
});
