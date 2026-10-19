/**
 * Turns the JSON of `EXPLAIN (FORMAT JSON)` into a tree that is easy to show: a readable
 * label per node, how much of the total each node accounts for by itself, and warnings for
 * the things people look for in a plan (bad row estimates, spills, discarded rows).
 */

export interface PlanBuffers {
  hit: number;
  read: number;
  dirtied: number;
  written: number;
  tempRead: number;
  tempWritten: number;
}

export interface PlanActual {
  startupMs: number;
  /** Time of one execution of the node, children included. */
  totalMs: number;
  /** Rows per execution. */
  rows: number;
  loops: number;
}

export interface PlanNode {
  id: number;
  depth: number;
  type: string;
  /** "Seq Scan on orders o" */
  label: string;
  /** How this node hangs below its parent: Outer, Inner, InitPlan, SubPlan, ... */
  relationship: string | null;
  startupCost: number;
  totalCost: number;
  planRows: number;
  planWidth: number;
  actual: PlanActual | null;
  /** Time spent in this node alone (children excluded), over all its loops. */
  selfMs: number | null;
  /** Cost of this node alone. */
  selfCost: number;
  /** Share of the whole statement: time when analysed, cost otherwise. 0..1 */
  share: number;
  /** actual/estimated rows when they are far apart (> 1 = planner guessed too low). */
  misestimate: number | null;
  buffers: PlanBuffers | null;
  details: { label: string; value: string }[];
  warnings: string[];
  children: PlanNode[];
}

export interface PlanTrigger {
  name: string;
  relation: string | null;
  calls: number;
  timeMs: number;
}

export interface ParsedPlan {
  root: PlanNode;
  analyzed: boolean;
  planningMs: number | null;
  executionMs: number | null;
  triggers: PlanTrigger[];
  /** All nodes, depth first (the order they are shown in). */
  nodes: PlanNode[];
  /** The node with the largest share. */
  heaviestId: number;
  /** Total cost of the root. */
  totalCost: number;
}

type Json = Record<string, unknown>;

const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const str = (v: unknown): string | null => (typeof v === 'string' && v !== '' ? v : null);

function label(p: Json): string {
  const type = str(p['Node Type']) ?? 'Node';
  // The JSON calls INSERT/UPDATE/DELETE/MERGE all "ModifyTable"; the text format says what it does.
  let out = type === 'ModifyTable' ? (str(p['Operation']) ?? type) : type;
  const join = str(p['Join Type']);
  // "Hash Join (Left)" says more than "Hash Join"; plain inner joins need no suffix.
  if (join && join !== 'Inner' && /Join|Loop/.test(type)) out += ` (${join})`;
  const strategy = str(p['Strategy']);
  if (type === 'Aggregate' && strategy === 'Hashed') out = 'HashAggregate';
  else if (type === 'Aggregate' && strategy === 'Sorted') out = 'GroupAggregate';
  const partial = str(p['Partial Mode']);
  if (partial && partial !== 'Simple') out = `${partial} ${out}`;
  const index = str(p['Index Name']);
  if (index) out += ` using ${index}`;
  const rel = str(p['Relation Name']);
  if (rel) {
    const schema = str(p['Schema']);
    out += ` on ${schema && schema !== 'public' ? `${schema}.` : ''}${rel}`;
    const alias = str(p['Alias']);
    if (alias && alias !== rel) out += ` ${alias}`;
  } else {
    const cte = str(p['CTE Name']);
    const fn = str(p['Function Name']);
    const alias = str(p['Alias']);
    if (cte) out += ` on ${cte}${alias && alias !== cte ? ` ${alias}` : ''}`;
    else if (fn) out += ` on ${fn}${alias && alias !== fn ? ` ${alias}` : ''}`;
    else if (alias && /Scan/.test(type)) out += ` on ${alias}`;
  }
  return out;
}

const joinList = (v: unknown) => (Array.isArray(v) ? v.join(', ') : String(v));

/** Properties worth showing when a node is expanded, in reading order. */
const DETAIL_KEYS: [string, string][] = [
  ['Scan Direction', 'Direction'],
  ['Index Cond', 'Index condition'],
  ['Recheck Cond', 'Recheck condition'],
  ['Filter', 'Filter'],
  ['Rows Removed by Filter', 'Rows removed by filter'],
  ['Rows Removed by Index Recheck', 'Rows removed by recheck'],
  ['Join Filter', 'Join filter'],
  ['Rows Removed by Join Filter', 'Rows removed by join filter'],
  ['Hash Cond', 'Hash condition'],
  ['Merge Cond', 'Merge condition'],
  ['Sort Key', 'Sort key'],
  ['Presorted Key', 'Presorted key'],
  ['Sort Method', 'Sort method'],
  ['Sort Space Used', 'Sort space (kB)'],
  ['Sort Space Type', 'Sort space type'],
  ['Group Key', 'Group key'],
  ['Hash Buckets', 'Hash buckets'],
  ['Hash Batches', 'Hash batches'],
  ['Peak Memory Usage', 'Peak memory (kB)'],
  ['Heap Fetches', 'Heap fetches'],
  ['Exact Heap Blocks', 'Exact heap blocks'],
  ['Lossy Heap Blocks', 'Lossy heap blocks'],
  ['Workers Planned', 'Workers planned'],
  ['Workers Launched', 'Workers launched'],
  ['Subplan Name', 'Subplan'],
  ['Output', 'Output'],
];

function buffersOf(p: Json): PlanBuffers | null {
  const b: PlanBuffers = {
    hit: num(p['Shared Hit Blocks']) ?? 0,
    read: num(p['Shared Read Blocks']) ?? 0,
    dirtied: num(p['Shared Dirtied Blocks']) ?? 0,
    written: num(p['Shared Written Blocks']) ?? 0,
    tempRead: num(p['Temp Read Blocks']) ?? 0,
    tempWritten: num(p['Temp Written Blocks']) ?? 0,
  };
  const any = Object.values(b).some((n) => n > 0);
  return 'Shared Hit Blocks' in p || any ? b : null;
}

function warningsOf(n: PlanNode, p: Json): string[] {
  const w: string[] = [];
  if (n.misestimate !== null) {
    const f = n.misestimate >= 1 ? n.misestimate : 1 / n.misestimate;
    w.push(
      n.misestimate >= 1
        ? `Planner expected ${fmtCount(n.planRows)} rows, got ${fmtCount(n.actual!.rows)} (${fmtFactor(f)} more)`
        : `Planner expected ${fmtCount(n.planRows)} rows, got ${fmtCount(n.actual!.rows)} (${fmtFactor(f)} fewer)`,
    );
  }
  const method = str(p['Sort Method']);
  if (method && /external|disk/i.test(method)) w.push(`Sort spilled to disk (${method})`);
  const batches = num(p['Hash Batches']);
  if (batches !== null && batches > 1) w.push(`Hash split into ${batches} batches (memory limit reached)`);
  else if (n.buffers && n.buffers.tempWritten > 0 && !method) w.push('Wrote temporary files');
  const removed = num(p['Rows Removed by Filter']);
  if (removed !== null && n.actual) {
    const discarded = removed * n.actual.loops;
    const kept = n.actual.rows * n.actual.loops;
    const seen = discarded + kept;
    if (discarded >= 1000 && seen > 0 && discarded / seen >= 0.9) {
      w.push(`Filter discarded ${fmtCount(discarded)} of ${fmtCount(seen)} rows read (${Math.round((discarded / seen) * 100)}%)`);
    }
  }
  const planned = num(p['Workers Planned']);
  const launched = num(p['Workers Launched']);
  if (planned !== null && launched !== null && launched < planned) {
    w.push(`Only ${launched} of ${planned} parallel workers started`);
  }
  const heap = num(p['Heap Fetches']);
  if (heap !== null && heap > 0 && /Index Only/.test(n.type)) w.push(`${fmtCount(heap)} heap fetches (visibility map not up to date)`);
  return w;
}

export function parsePlan(json: string): ParsedPlan {
  let doc: unknown;
  try {
    doc = JSON.parse(json);
  } catch {
    throw new Error('The server returned a plan that could not be read.');
  }
  const top = (Array.isArray(doc) ? doc[0] : doc) as Json | undefined;
  const rootJson = top?.['Plan'] as Json | undefined;
  if (!top || !rootJson || typeof rootJson !== 'object') throw new Error('The server returned no plan.');

  const nodes: PlanNode[] = [];
  let analyzed = false;

  const build = (p: Json, depth: number): PlanNode => {
    const startedAt = num(p['Actual Total Time']);
    const actual: PlanActual | null =
      startedAt === null
        ? null
        : {
            startupMs: num(p['Actual Startup Time']) ?? 0,
            totalMs: startedAt,
            rows: num(p['Actual Rows']) ?? 0,
            loops: num(p['Actual Loops']) ?? 1,
          };
    if (actual) analyzed = true;

    const node: PlanNode = {
      id: nodes.length,
      depth,
      type: str(p['Node Type']) ?? 'Node',
      label: label(p),
      relationship: str(p['Parent Relationship']),
      startupCost: num(p['Startup Cost']) ?? 0,
      totalCost: num(p['Total Cost']) ?? 0,
      planRows: num(p['Plan Rows']) ?? 0,
      planWidth: num(p['Plan Width']) ?? 0,
      actual,
      selfMs: null,
      selfCost: 0,
      share: 0,
      misestimate: null,
      buffers: buffersOf(p),
      details: [],
      warnings: [],
      children: [],
    };
    nodes.push(node);

    for (const [key, name] of DETAIL_KEYS) {
      if (p[key] !== undefined && p[key] !== null && !(Array.isArray(p[key]) && (p[key] as unknown[]).length === 0)) {
        node.details.push({ label: name, value: joinList(p[key]) });
      }
    }
    for (const c of (p['Plans'] as Json[] | undefined) ?? []) node.children.push(build(c, depth + 1));
    return node;
  };

  const root = build(rootJson, 0);

  // Exclusive numbers: a node's own work is what is left after its children.
  const totalOf = (n: PlanNode) => (n.actual ? n.actual.totalMs * n.actual.loops : 0);
  for (const n of nodes) {
    const childCost = n.children.reduce((s, c) => s + c.totalCost, 0);
    n.selfCost = Math.max(n.totalCost - childCost, 0);
    if (n.actual) {
      const childMs = n.children.reduce((s, c) => s + totalOf(c), 0);
      n.selfMs = Math.max(totalOf(n) - childMs, 0);
      // Per-loop numbers on both sides. Tiny counts are noise, so compare against at least 1.
      const f = Math.max(n.actual.rows, 1) / Math.max(n.planRows, 1);
      if ((f >= 10 || f <= 0.1) && n.actual.loops > 0) n.misestimate = f;
    }
  }
  const rootMs = totalOf(root);
  for (const n of nodes) {
    n.share = analyzed
      ? rootMs > 0
        ? (n.selfMs ?? 0) / rootMs
        : 0
      : root.totalCost > 0
        ? n.selfCost / root.totalCost
        : 0;
    n.share = Math.min(Math.max(n.share, 0), 1);
  }
  // Warnings need the raw JSON again; walk both trees in the same order.
  let i = 0;
  const walk = (p: Json) => {
    const n = nodes[i++];
    n.warnings = warningsOf(n, p);
    for (const c of (p['Plans'] as Json[] | undefined) ?? []) walk(c);
  };
  walk(rootJson);

  let heaviest = nodes[0];
  for (const n of nodes) if (n.share > heaviest.share) heaviest = n;

  const triggers = ((top['Triggers'] as Json[] | undefined) ?? []).map((t) => ({
    name: str(t['Trigger Name']) ?? '(trigger)',
    relation: str(t['Relation']),
    calls: num(t['Calls']) ?? 0,
    timeMs: num(t['Time']) ?? 0,
  }));

  return {
    root,
    analyzed,
    planningMs: num(top['Planning Time']),
    executionMs: num(top['Execution Time']),
    triggers,
    nodes,
    heaviestId: heaviest.id,
    totalCost: root.totalCost,
  };
}

/* ------------------------------------------------------------------ formatting */

export function fmtMs(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)} s`;
  if (ms >= 100) return `${Math.round(ms)} ms`;
  if (ms >= 1) return `${ms.toFixed(1)} ms`;
  if (ms === 0) return '0 ms';
  return `${ms.toFixed(3)} ms`;
}

export function fmtCount(n: number): string {
  if (Number.isInteger(n)) return n.toLocaleString('en-US');
  return n.toLocaleString('en-US', { maximumFractionDigits: 1 });
}

export function fmtFactor(f: number): string {
  return f >= 100 ? `${Math.round(f).toLocaleString('en-US')}×` : f >= 10 ? `${Math.round(f)}×` : `${f.toFixed(1)}×`;
}

export function fmtCost(c: number): string {
  return c >= 1000 ? Math.round(c).toLocaleString('en-US') : c.toFixed(2);
}

export function fmtPercent(share: number): string {
  const p = share * 100;
  if (p === 0) return '0%';
  return p < 1 ? '<1%' : `${Math.round(p)}%`;
}
