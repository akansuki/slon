import { describe, expect, it } from 'vitest';
import { fmtCount, fmtMs, fmtPercent, parsePlan } from './plan';
import analyzeJoin from './fixtures/analyze-join.json?raw';
import plain from './fixtures/plain.json?raw';
import spill from './fixtures/spill.json?raw';
import filter from './fixtures/filter.json?raw';

describe('parsePlan', () => {
  it('reads an analysed join: tree shape, labels, timings', () => {
    const p = parsePlan(analyzeJoin);
    expect(p.analyzed).toBe(true);
    expect(p.executionMs).toBeGreaterThan(0);
    expect(p.planningMs).toBeGreaterThan(0);
    expect(p.root.type).toBe('Sort');
    expect(p.nodes.map((n) => n.type)).toEqual(['Sort', 'Aggregate', 'Hash Join', 'Seq Scan', 'Hash', 'Seq Scan']);
    expect(p.nodes.map((n) => n.id)).toEqual([0, 1, 2, 3, 4, 5]);
    expect(p.nodes.map((n) => n.depth)).toEqual([0, 1, 2, 3, 3, 4]);
    expect(p.nodes[3].label).toBe('Seq Scan on o');
    expect(p.nodes[5].label).toBe('Seq Scan on c');
    expect(p.nodes[1].label).toBe('HashAggregate');
    expect(p.nodes[3].relationship).toBe('Outer');
  });

  it('computes exclusive time that adds up to the root and flags the heaviest node', () => {
    const p = parsePlan(analyzeJoin);
    const self = p.nodes.reduce((s, n) => s + (n.selfMs ?? 0), 0);
    const rootTotal = p.root.actual!.totalMs * p.root.actual!.loops;
    // Every node's own time, summed, is the root's total (rounding aside).
    expect(self).toBeCloseTo(rootTotal, 1);
    expect(p.nodes.reduce((s, n) => s + n.share, 0)).toBeCloseTo(1, 1);
    const max = Math.max(...p.nodes.map((n) => n.share));
    expect(p.nodes[p.heaviestId].share).toBe(max);
  });

  it('plain EXPLAIN has costs but no timings, and shares are by cost', () => {
    const p = parsePlan(plain);
    expect(p.analyzed).toBe(false);
    expect(p.executionMs).toBeNull();
    expect(p.root.actual).toBeNull();
    expect(p.root.selfMs).toBeNull();
    expect(p.totalCost).toBeGreaterThan(0);
    expect(p.nodes.reduce((s, n) => s + n.share, 0)).toBeCloseTo(1, 3);
    expect(p.nodes.every((n) => n.warnings.length === 0)).toBe(true);
    expect(p.root.type).toBe('Limit');
  });

  it('warns about a sort that spilled to disk', () => {
    const p = parsePlan(spill);
    const sort = p.nodes.find((n) => n.type === 'Sort')!;
    expect(sort.warnings.some((w) => /spilled to disk/i.test(w))).toBe(true);
    expect(sort.details.find((d) => d.label === 'Sort key')?.value).toBe('note, amount');
    expect(sort.buffers?.tempWritten).toBeGreaterThan(0);
  });

  it('shows filters and flags a large mis-estimate', () => {
    const p = parsePlan(filter);
    const scan = p.nodes.find((n) => n.type === 'Seq Scan')!;
    expect(scan.details.find((d) => d.label === 'Filter')?.value).toContain('zzz');
    expect(scan.details.find((d) => d.label === 'Rows removed by filter')).toBeTruthy();
  });

  it('names data-changing nodes after what they do', () => {
    const p = parsePlan(
      JSON.stringify([{ Plan: { 'Node Type': 'ModifyTable', Operation: 'Delete', 'Relation Name': 't', Alias: 't', 'Total Cost': 1 } }]),
    );
    expect(p.root.label).toBe('Delete on t');
  });

  it('rejects things that are not plans', () => {
    expect(() => parsePlan('nope')).toThrow(/could not be read/);
    expect(() => parsePlan('[]')).toThrow(/no plan/);
    expect(() => parsePlan('[{"Planning Time": 1}]')).toThrow(/no plan/);
  });

  it('handles synthetic cases: loops, left joins, misestimates, trigger timings', () => {
    const json = JSON.stringify([
      {
        Plan: {
          'Node Type': 'Nested Loop',
          'Join Type': 'Left',
          'Total Cost': 100,
          'Plan Rows': 5,
          'Actual Total Time': 10,
          'Actual Rows': 500,
          'Actual Loops': 1,
          Plans: [
            {
              'Node Type': 'Seq Scan',
              'Parent Relationship': 'Outer',
              'Relation Name': 'a',
              Alias: 'x',
              Schema: 'sales',
              'Total Cost': 10,
              'Plan Rows': 500,
              'Actual Total Time': 1,
              'Actual Rows': 500,
              'Actual Loops': 1,
            },
            {
              'Node Type': 'Index Scan',
              'Parent Relationship': 'Inner',
              'Relation Name': 'b',
              'Index Name': 'b_pkey',
              'Total Cost': 0.3,
              'Plan Rows': 1,
              'Actual Total Time': 0.012,
              'Actual Rows': 1,
              'Actual Loops': 500,
            },
          ],
        },
        'Planning Time': 0.2,
        'Execution Time': 10.5,
        Triggers: [{ 'Trigger Name': 'trg', Relation: 'a', Time: 3.5, Calls: 500 }],
      },
    ]);
    const p = parsePlan(json);
    expect(p.root.label).toBe('Nested Loop (Left)');
    expect(p.nodes[1].label).toBe('Seq Scan on sales.a x');
    expect(p.nodes[2].label).toBe('Index Scan using b_pkey on b');
    // Inner side ran 500 times: 0.012 ms * 500 = 6 ms, which is what counts.
    expect(p.nodes[2].selfMs).toBeCloseTo(6, 5);
    expect(p.root.selfMs).toBeCloseTo(3, 5); // 10 - 1 - 6
    expect(p.root.misestimate).toBe(100);
    expect(p.root.warnings[0]).toMatch(/expected 5 rows, got 500 \(100× more\)/);
    expect(p.triggers).toEqual([{ name: 'trg', relation: 'a', calls: 500, timeMs: 3.5 }]);
  });
});

describe('formatting', () => {
  it('formats durations, counts and shares', () => {
    expect(fmtMs(0.0123)).toBe('0.012 ms');
    expect(fmtMs(1.234)).toBe('1.2 ms');
    expect(fmtMs(250.6)).toBe('251 ms');
    expect(fmtMs(2500)).toBe('2.50 s');
    expect(fmtCount(1234567)).toBe('1,234,567');
    expect(fmtPercent(0)).toBe('0%');
    expect(fmtPercent(0.004)).toBe('<1%');
    expect(fmtPercent(0.456)).toBe('46%');
  });
});
