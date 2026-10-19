<script lang="ts">
  import Icon from './Icon.svelte';
  import { fmtCost, fmtCount, fmtMs, fmtPercent, type ParsedPlan, type PlanNode } from './plan';
  import { copyText } from './selection';
  import { app } from './store.svelte';

  let { plan, json, sql }: { plan: ParsedPlan; json: string; sql: string } = $props();

  // Per node: children folded away / details unfolded.
  let folded = $state<Record<number, boolean>>({});
  let opened = $state<Record<number, boolean>>({});

  // A new plan starts with a clean slate.
  $effect(() => {
    void plan;
    folded = {};
    opened = {};
  });

  const visible = $derived.by(() => {
    const out: PlanNode[] = [];
    let hideBelow: number | null = null;
    for (const n of plan.nodes) {
      if (hideBelow !== null) {
        if (n.depth > hideBelow) continue;
        hideBelow = null;
      }
      out.push(n);
      if (folded[n.id] && n.children.length > 0) hideBelow = n.depth;
    }
    return out;
  });

  const warningCount = $derived(plan.nodes.reduce((s, n) => s + n.warnings.length, 0));

  function barColor(share: number) {
    return share >= 0.5 ? 'var(--danger)' : share >= 0.2 ? 'var(--warn)' : 'var(--accent)';
  }

  function rowsText(n: PlanNode) {
    return n.actual
      ? `${fmtCount(n.planRows)} → ${fmtCount(n.actual.rows)}`
      : fmtCount(n.planRows);
  }

  function toggleFold(e: MouseEvent, n: PlanNode) {
    e.stopPropagation();
    folded[n.id] = !folded[n.id];
  }

  function foldAll(fold: boolean) {
    const next: Record<number, boolean> = {};
    if (fold) for (const n of plan.nodes) if (n.children.length > 0 && n.depth > 0) next[n.id] = true;
    folded = next;
  }

  async function copyJson() {
    app.flash((await copyText(json)) ? 'Copied plan as JSON' : 'Copy failed: clipboard not available');
  }

  function buffersText(n: PlanNode) {
    const b = n.buffers;
    if (!b) return '';
    const parts = [`${fmtCount(b.hit)} hit`, `${fmtCount(b.read)} read`];
    if (b.dirtied) parts.push(`${fmtCount(b.dirtied)} dirtied`);
    if (b.written) parts.push(`${fmtCount(b.written)} written`);
    if (b.tempRead || b.tempWritten) parts.push(`temp ${fmtCount(b.tempRead)} read / ${fmtCount(b.tempWritten)} written`);
    return parts.join(' · ');
  }
</script>

<div class="plan" data-testid="plan-view">
  <div class="summary" data-testid="plan-summary">
    {#if plan.analyzed}
      <span class="stat"><b>{plan.executionMs !== null ? fmtMs(plan.executionMs) : '–'}</b> execution</span>
      {#if plan.planningMs !== null}<span class="stat"><b>{fmtMs(plan.planningMs)}</b> planning</span>{/if}
    {:else}
      <span class="stat"><b>{fmtCost(plan.totalCost)}</b> estimated total cost</span>
      {#if plan.planningMs !== null}<span class="stat"><b>{fmtMs(plan.planningMs)}</b> planning</span>{/if}
    {/if}
    <span class="stat"><b>{plan.nodes.length}</b> node{plan.nodes.length === 1 ? '' : 's'}</span>
    {#if warningCount > 0}
      <span class="stat warn"><Icon name="alert" size={13} /> <b>{warningCount}</b> warning{warningCount === 1 ? '' : 's'}</span>
    {/if}
    <span class="spacer"></span>
    <button class="ghost" onclick={() => foldAll(false)} title="Show every node">Expand all</button>
    <button class="ghost" onclick={() => foldAll(true)} title="Fold everything below the first level">Collapse</button>
    <button class="ghost" onclick={copyJson} title="Copy the plan as the server's JSON document" data-testid="plan-copy-json">
      <Icon name="copy" size={14} /> Copy JSON
    </button>
  </div>

  {#if !plan.analyzed}
    <div class="note">
      Estimates only: the statement was not run. Use <b>Explain analyze</b> for real timings and row counts.
    </div>
  {/if}

  <div class="tablewrap">
    <div class="head" role="presentation">
      <span class="c-node">Node</span>
      <span class="c-rows" title={plan.analyzed ? 'Rows estimated by the planner → rows actually produced (per loop)' : 'Rows estimated by the planner'}>
        Rows{plan.analyzed ? ' (est → actual)' : ' (est)'}
      </span>
      <span class="c-self" title={plan.analyzed ? 'Time spent in the node itself, children not counted, summed over all loops' : 'Planner cost of the node itself, children not counted'}>
        {plan.analyzed ? 'Self time' : 'Self cost'}
      </span>
      <span class="c-bar" title="Share of the whole statement"></span>
    </div>

    <div class="nodes" role="tree">
      {#each visible as n (n.id)}
        <div class="node" class:open={opened[n.id]} data-testid="plan-node">
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="row"
            class:heavy={n.id === plan.heaviestId && n.share >= 0.2}
            role="treeitem"
            aria-selected={!!opened[n.id]}
            tabindex="0"
            onclick={() => (opened[n.id] = !opened[n.id])}
            onkeydown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                opened[n.id] = !opened[n.id];
              }
            }}
          >
            <span class="c-node" style:padding-left="{n.depth * 18 + 4}px">
              {#if n.children.length > 0}
                <button class="fold" class:folded={folded[n.id]} onclick={(e) => toggleFold(e, n)} aria-label={folded[n.id] ? 'Show children' : 'Hide children'}>
                  <Icon name="chevron" size={12} />
                </button>
              {:else}
                <span class="fold-gap"></span>
              {/if}
              <span class="label" data-testid="plan-label">{n.label}</span>
              {#if n.relationship && n.relationship !== 'Outer' && n.relationship !== 'Inner' && n.relationship !== 'Member'}
                <span class="tag">{n.relationship}</span>
              {/if}
              {#if n.actual && n.actual.loops > 1}<span class="tag" title="The node ran this many times">× {fmtCount(n.actual.loops)} loops</span>{/if}
              {#if n.warnings.length > 0}
                <span class="warnmark" title={n.warnings.join('\n')} data-testid="plan-warning"><Icon name="alert" size={13} /></span>
              {/if}
            </span>
            <span class="c-rows num" class:off={n.misestimate !== null}>{rowsText(n)}</span>
            <span class="c-self num">
              {#if n.selfMs !== null}{fmtMs(n.selfMs)}{:else}{fmtCost(n.selfCost)}{/if}
            </span>
            <span class="c-bar">
              <span class="bar"><span class="fill" style:width="{Math.max(n.share * 100, n.share > 0 ? 2 : 0)}%" style:background={barColor(n.share)}></span></span>
              <span class="pct">{fmtPercent(n.share)}</span>
            </span>
          </div>

          {#if opened[n.id]}
            <div class="detail" data-testid="plan-detail">
              {#each n.warnings as w}
                <div class="warnline"><Icon name="alert" size={13} /> {w}</div>
              {/each}
              <dl>
                <dt>Cost</dt>
                <dd>{fmtCost(n.startupCost)} .. {fmtCost(n.totalCost)} <span class="dim">(startup .. total, children included)</span></dd>
                <dt>Rows</dt>
                <dd>
                  {fmtCount(n.planRows)} estimated{#if n.actual}, {fmtCount(n.actual.rows)} actual{/if} <span class="dim">· width {n.planWidth} B</span>
                </dd>
                {#if n.actual}
                  <dt>Time</dt>
                  <dd>
                    {fmtMs(n.actual.startupMs)} .. {fmtMs(n.actual.totalMs)} per loop
                    {#if n.actual.loops > 1}<span class="dim">× {fmtCount(n.actual.loops)} loops</span>{/if}
                  </dd>
                {/if}
                {#if n.buffers}
                  <dt>Buffers</dt>
                  <dd>{buffersText(n)} <span class="dim">(blocks)</span></dd>
                {/if}
                {#each n.details as d}
                  <dt>{d.label}</dt>
                  <dd class="code">{d.value}</dd>
                {/each}
              </dl>
            </div>
          {/if}
        </div>
      {/each}
    </div>

    {#if plan.triggers.length > 0}
      <div class="triggers">
        <b>Triggers</b>
        {#each plan.triggers as t}
          <div>{t.name}{t.relation ? ` on ${t.relation}` : ''}: {fmtMs(t.timeMs)} over {fmtCount(t.calls)} call{t.calls === 1 ? '' : 's'}</div>
        {/each}
      </div>
    {/if}
  </div>

  <details class="sql">
    <summary>Statement</summary>
    <pre>{sql}</pre>
  </details>
</div>

<style>
  .plan {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--panel);
  }
  .summary {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--panel-2);
    flex-wrap: wrap;
  }
  .stat {
    color: var(--muted);
    display: inline-flex;
    gap: 4px;
    align-items: center;
  }
  .stat b {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .stat.warn {
    color: var(--warn);
  }
  .spacer {
    flex: 1;
  }
  .note {
    padding: 6px 12px;
    background: var(--warn-bg);
    color: var(--warn);
    border-bottom: 1px solid var(--border);
    font-size: 12px;
  }
  .tablewrap {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .head,
  .row {
    display: grid;
    grid-template-columns: minmax(280px, 1fr) 150px 90px 150px;
    align-items: center;
    min-width: 700px;
  }
  .head {
    position: sticky;
    top: 0;
    z-index: 2;
    background: var(--panel-2);
    border-bottom: 1px solid var(--border);
    color: var(--muted);
    font-size: 12px;
    padding: 4px 0;
  }
  .head > span {
    padding: 0 10px;
  }
  .c-rows,
  .c-self {
    text-align: right;
  }
  .row {
    padding: 4px 0;
    border-bottom: 1px solid var(--border);
    cursor: pointer;
  }
  .row:hover {
    background: var(--hover);
  }
  .row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .row.heavy {
    background: var(--stripe);
  }
  .open > .row {
    background: var(--selected);
  }
  .row > span {
    padding: 0 10px;
    min-width: 0;
  }
  .c-node {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
  }
  .label {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tag {
    flex: none;
    font-size: 11px;
    padding: 0 6px;
    border-radius: 8px;
    background: var(--panel-2);
    color: var(--muted);
    border: 1px solid var(--border);
  }
  .warnmark {
    flex: none;
    color: var(--warn);
    display: inline-flex;
  }
  .num {
    font-family: var(--mono);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .num.off {
    color: var(--danger);
    font-weight: 600;
  }
  .fold {
    all: unset;
    display: inline-flex;
    flex: none;
    width: 16px;
    height: 16px;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: var(--muted);
    transform: rotate(90deg);
  }
  .fold.folded {
    transform: none;
  }
  .fold:hover {
    color: var(--text);
  }
  .fold-gap {
    flex: none;
    width: 16px;
  }
  .c-bar {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .bar {
    flex: 1;
    height: 8px;
    border-radius: 4px;
    background: var(--panel-2);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
  }
  .pct {
    width: 34px;
    text-align: right;
    font-size: 12px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .detail {
    padding: 8px 14px 10px 44px;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
    min-width: 700px;
  }
  .warnline {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--warn);
    margin-bottom: 4px;
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 3px 14px;
    margin: 4px 0 0;
  }
  dt {
    color: var(--muted);
    font-size: 12px;
  }
  dd {
    margin: 0;
    min-width: 0;
    word-break: break-word;
  }
  dd.code {
    font-family: var(--mono);
    font-size: 12px;
  }
  .dim {
    color: var(--muted);
  }
  .triggers {
    padding: 8px 14px;
    color: var(--muted);
  }
  .sql {
    border-top: 1px solid var(--border);
    background: var(--panel-2);
    padding: 4px 12px;
    font-size: 12px;
    color: var(--muted);
  }
  .sql summary {
    cursor: pointer;
  }
  .sql pre {
    margin: 6px 0;
    max-height: 160px;
    overflow: auto;
    font-family: var(--mono);
    color: var(--text);
    white-space: pre-wrap;
  }
</style>
