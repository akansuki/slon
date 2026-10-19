<script lang="ts">
  import { nextSort, sortedOrder, type SortSpec } from './sort';
  import {
    copyText,
    describe,
    inRange,
    toRange,
    toText,
    type Pos,
    type Selection,
  } from './selection';
  import type { Cell, Column } from './types';

  let {
    columns,
    rows,
    version,
    running,
    sort = $bindable(null),
  }: { columns: Column[]; rows: Cell[][]; version: number; running: boolean; sort?: SortSpec | null } = $props();

  const ROW_H = 26;
  const HEADER_H = 34;
  const OVERSCAN = 10;
  const NUM_W = 64;
  const MIN_COL_W = 40;
  const MAX_COL_W = 2000;

  let scrollTop = $state(0);
  let viewH = $state(0);
  let scroller = $state<HTMLDivElement>();
  let bodyEl = $state<HTMLDivElement>();

  let sel = $state.raw<Selection | null>(null);
  /** Column widths the user dragged; `undefined` entries keep the automatic width. */
  let custom = $state.raw<(number | undefined)[]>([]);
  let toast = $state<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let menu = $state<{ x: number; y: number } | null>(null);

  // A different result set starts unsorted, unselected, at the top, with automatic widths.
  $effect(() => {
    void columns;
    sort = null;
    sel = null;
    custom = [];
    menu = null;
    if (scroller) {
      scroller.scrollTop = 0;
      scroller.scrollLeft = 0;
      scrollTop = 0;
    }
  });

  // Once rows stop streaming a pending sort is applied: indexes no longer match the selection.
  $effect(() => {
    if (!running && sort) sel = null;
  });

  // `rows` is mutated in place while streaming, `version` is the change signal.
  const count = $derived.by(() => {
    void version;
    return rows.length;
  });

  // Rows are never moved: sorting yields an index permutation. While rows are still
  // streaming in, the arrival order is shown and the sort applies once the query ends.
  const order = $derived.by(() => {
    void version;
    if (!sort || running) return null;
    return sortedOrder(rows, sort.col, columns[sort.col]?.sortKind ?? 'unknown', sort.dir);
  });

  const autoWidths = $derived.by(() => {
    void version;
    const sample = rows.slice(0, 200);
    return columns.map((c, i) => {
      let chars = Math.max(c.name.length, (c.typeName ?? '').length) + 3;
      for (const r of sample) {
        const v = r[i];
        chars = Math.max(chars, v === null ? 4 : Math.min(v.length, 60));
      }
      return Math.max(90, Math.min(Math.round(chars * 7.4 + 26), 440));
    });
  });
  const widths = $derived(autoWidths.map((w, i) => custom[i] ?? w));

  const totalWidth = $derived(NUM_W + widths.reduce((a, b) => a + b, 0));
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN));
  const last = $derived(Math.min(count, Math.ceil((scrollTop + viewH) / ROW_H) + OVERSCAN));
  const visible = $derived.by(() => {
    const out: number[] = [];
    for (let i = first; i < last; i++) out.push(i);
    return out;
  });

  const range = $derived(sel ? toRange(sel, columns.length) : null);

  function ariaSort(i: number) {
    return sort?.col === i ? (sort.dir === 'asc' ? 'ascending' : 'descending') : 'none';
  }

  function onSort(i: number) {
    sort = nextSort(sort, i);
    sel = null; // the same rows now sit at different positions
  }

  const valueAt = (r: number, c: number): Cell => rows[order ? order[r] : r]?.[c] ?? null;

  /* ------------------------------------------------------------------ column resize */

  function setWidth(i: number, w: number) {
    const next = custom.slice();
    next[i] = Math.max(MIN_COL_W, Math.min(Math.round(w), MAX_COL_W));
    custom = next;
  }

  function startResize(e: MouseEvent, i: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const startW = widths[i];
    const move = (ev: MouseEvent) => setWidth(i, startW + ev.clientX - startX);
    const up = () => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  }

  let measureCtx: CanvasRenderingContext2D | null = null;
  function textWidth(text: string): number {
    if (!measureCtx) {
      measureCtx = document.createElement('canvas').getContext('2d');
      if (measureCtx && scroller) measureCtx.font = `12.5px ${getComputedStyle(scroller).fontFamily}`;
    }
    return measureCtx ? measureCtx.measureText(text).width : text.length * 7.4;
  }

  /** Double-click on a column border: fit the column to its content. */
  function autoFit(i: number) {
    const c = columns[i];
    let w = Math.max(c.name.length * 8, (c.typeName ?? '').length * 6.5) + 40;
    const n = Math.min(rows.length, 5000);
    for (let r = 0; r < n; r++) {
      const v = rows[r][i];
      w = Math.max(w, (v === null ? 4 * 7.4 : textWidth(v.length > 300 ? v.slice(0, 300) : v)) + 24);
    }
    setWidth(i, w);
  }

  /* ------------------------------------------------------------------ selection */

  function columnAtX(x: number): number {
    let acc = 0;
    for (let i = 0; i < widths.length; i++) {
      acc += widths[i];
      if (x < acc) return i;
    }
    return Math.max(widths.length - 1, 0);
  }

  /** Grid position under a viewport point (clamped to the rows and columns that exist). */
  function posAt(clientX: number, clientY: number): { pos: Pos; numCol: boolean } | null {
    if (!scroller || !bodyEl || count === 0 || columns.length === 0) return null;
    const g = scroller.getBoundingClientRect();
    const b = bodyEl.getBoundingClientRect();
    const r = Math.max(0, Math.min(Math.floor((clientY - b.top) / ROW_H), count - 1));
    const numCol = clientX - g.left < NUM_W;
    const c = columnAtX(Math.max(0, clientX - b.left - NUM_W));
    return { pos: { r, c }, numCol };
  }

  function select(next: Selection) {
    sel = next;
  }

  function onBodyMouseDown(e: MouseEvent) {
    if (e.button !== 0 && !(e.button === 2 && !selectionContains(e))) return;
    const hit = posAt(e.clientX, e.clientY);
    if (!hit) return;
    menu = null;
    scroller?.focus({ preventScroll: true });
    if (e.button === 2) {
      select({ anchor: hit.pos, focus: hit.pos, rows: hit.numCol });
      return;
    }
    e.preventDefault(); // no native text selection while dragging
    if (e.shiftKey && sel) {
      select({ anchor: sel.anchor, focus: hit.pos, rows: sel.rows || hit.numCol });
    } else {
      select({ anchor: hit.pos, focus: hit.pos, rows: hit.numCol });
    }
    const rowsMode = hit.numCol || (e.shiftKey && !!sel?.rows);
    const move = (ev: MouseEvent) => {
      if (!scroller) return;
      const g = scroller.getBoundingClientRect();
      // Scroll while dragging near or past the edge.
      if (ev.clientY > g.bottom - 24) scroller.scrollTop += 24;
      else if (ev.clientY < g.top + HEADER_H + 12) scroller.scrollTop -= 24;
      if (ev.clientX > g.right - 24) scroller.scrollLeft += 24;
      else if (ev.clientX < g.left + NUM_W + 12) scroller.scrollLeft -= 24;
      const h = posAt(ev.clientX, ev.clientY);
      if (h && sel) select({ anchor: sel.anchor, focus: h.pos, rows: rowsMode });
    };
    const up = () => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  }

  function selectionContains(e: MouseEvent): boolean {
    const hit = posAt(e.clientX, e.clientY);
    return !!(hit && range && inRange(range, hit.pos.r, hit.numCol ? range.c0 : hit.pos.c));
  }

  function scrollIntoView(p: Pos) {
    if (!scroller) return;
    const top = p.r * ROW_H;
    if (top < scroller.scrollTop) scroller.scrollTop = top;
    else if (top + ROW_H > scroller.scrollTop + viewH - HEADER_H) {
      scroller.scrollTop = top + ROW_H - (viewH - HEADER_H);
    }
    let left = 0;
    for (let i = 0; i < p.c; i++) left += widths[i];
    const w = widths[p.c] ?? 0;
    const visibleW = scroller.clientWidth - NUM_W;
    if (left < scroller.scrollLeft) scroller.scrollLeft = left;
    else if (left + w > scroller.scrollLeft + visibleW) scroller.scrollLeft = Math.min(left, left + w - visibleW);
  }

  async function copy(withHeaders: boolean) {
    if (!range) return;
    const text = toText({ columns: columns.map((c) => c.name), cell: valueAt }, range, withHeaders);
    const ok = await copyText(text);
    flash(ok ? `Copied ${describe(range)}${withHeaders ? ' with headers' : ''}` : 'Copy failed: clipboard not available');
  }

  function flash(message: string) {
    toast = message;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 1800);
  }

  function onKeyDown(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    const key = e.key;
    if (mod && key.toLowerCase() === 'c') {
      if (range) {
        e.preventDefault();
        void copy(e.shiftKey);
      }
      return;
    }
    if (mod && key.toLowerCase() === 'a') {
      if (count > 0 && columns.length > 0) {
        e.preventDefault();
        select({ anchor: { r: 0, c: 0 }, focus: { r: count - 1, c: columns.length - 1 }, rows: false });
      }
      return;
    }
    if (key === 'Escape') {
      menu = null;
      sel = null;
      return;
    }
    if (count === 0 || columns.length === 0) return;
    const cur: Pos = sel?.focus ?? { r: 0, c: 0 };
    let { r, c } = cur;
    const page = Math.max(1, Math.floor((viewH - HEADER_H) / ROW_H) - 1);
    switch (key) {
      case 'ArrowDown': r += 1; break;
      case 'ArrowUp': r -= 1; break;
      case 'ArrowRight': c += 1; break;
      case 'ArrowLeft': c -= 1; break;
      case 'PageDown': r += page; break;
      case 'PageUp': r -= page; break;
      case 'Home': if (mod) r = 0; c = 0; break;
      case 'End': if (mod) r = count - 1; c = columns.length - 1; break;
      default: return;
    }
    e.preventDefault();
    const next: Pos = {
      r: Math.max(0, Math.min(r, count - 1)),
      c: Math.max(0, Math.min(c, columns.length - 1)),
    };
    if (e.shiftKey && sel) select({ anchor: sel.anchor, focus: next, rows: sel.rows });
    else select({ anchor: next, focus: next, rows: false });
    scrollIntoView(next);
  }

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    if (!range) return;
    // Keep the menu inside the window.
    menu = { x: Math.min(e.clientX, window.innerWidth - 190), y: Math.min(e.clientY, window.innerHeight - 90) };
  }

  function closeMenu() {
    menu = null;
  }
</script>

<svelte:window onmousedown={(e) => { if (menu && !(e.target as HTMLElement).closest('.ctxmenu')) closeMenu(); }} onblur={closeMenu} />

<div class="gridwrap">
  <div
    class="grid"
    bind:this={scroller}
    bind:clientHeight={viewH}
    onscroll={(e) => {
      scrollTop = e.currentTarget.scrollTop;
      menu = null;
    }}
    onkeydown={onKeyDown}
    oncontextmenu={onContextMenu}
    role="grid"
    aria-rowcount={count + 1}
    aria-multiselectable="true"
    tabindex="0"
  >
    <div class="header" style:width="{totalWidth}px" role="row">
      <div class="cell num" style:width="{NUM_W}px"></div>
      {#each columns as c, i (i)}
        <div class="hcell" style:width="{widths[i]}px">
          <button
            class="head"
            class:numeric={c.sortKind === 'number'}
            onclick={() => onSort(i)}
            title="{c.name}{c.typeName ? ` (${c.typeName})` : ''} – click to sort"
            role="columnheader"
            aria-sort={ariaSort(i)}
          >
            <span class="label">
              <span class="name">{c.name}</span>
              {#if c.typeName}<span class="type">{c.typeName}</span>{/if}
            </span>
            {#if sort?.col === i}
              <span class="arrow">{sort.dir === 'asc' ? '▲' : '▼'}</span>
            {/if}
          </button>
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <span
            class="resizer"
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize column {c.name}"
            title="Drag to resize, double-click to fit"
            onmousedown={(e) => startResize(e, i)}
            ondblclick={() => autoFit(i)}
          ></span>
        </div>
      {/each}
    </div>

    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="body"
      bind:this={bodyEl}
      style:height="{count * ROW_H}px"
      style:width="{totalWidth}px"
      onmousedown={onBodyMouseDown}
    >
      {#each visible as i (i)}
        {@const row = rows[order ? order[i] : i]}
        {#if row}
          <div class="row" class:stripe={i % 2 === 1} style:top="{i * ROW_H}px" role="row">
            <div
              class="cell num"
              class:rowsel={sel?.rows && range && i >= range.r0 && i <= range.r1}
              style:width="{NUM_W}px"
            >{(i + 1).toLocaleString()}</div>
            {#each columns as c, ci (ci)}
              {@const v = row[ci]}
              <div
                class="cell"
                class:numeric={c.sortKind === 'number'}
                class:sel={range && inRange(range, i, ci)}
                class:focus={sel && sel.focus.r === i && sel.focus.c === ci && !sel.rows}
                style:width="{widths[ci]}px"
                title={v === null ? 'NULL' : v.length > 400 ? v.slice(0, 400) + '…' : v}
                role="gridcell"
                aria-selected={range ? inRange(range, i, ci) : false}
              >
                {#if v === null}<span class="null">NULL</span>{:else}{v}{/if}
              </div>
            {/each}
          </div>
        {/if}
      {/each}
    </div>
  </div>

  {#if toast}<div class="toast" role="status" data-testid="copy-toast">{toast}</div>{/if}

  {#if menu}
    <div class="ctxmenu" style:left="{menu.x}px" style:top="{menu.y}px" role="menu">
      <button role="menuitem" data-testid="ctx-copy" onclick={() => { closeMenu(); void copy(false); }}>
        Copy <kbd>Ctrl+C</kbd>
      </button>
      <button role="menuitem" data-testid="ctx-copy-headers" onclick={() => { closeMenu(); void copy(true); }}>
        Copy with headers <kbd>Ctrl+Shift+C</kbd>
      </button>
    </div>
  {/if}
</div>

<style>
  .gridwrap {
    position: relative;
    height: 100%;
  }
  .grid {
    position: relative;
    height: 100%;
    overflow: auto;
    background: var(--panel);
    font-family: var(--mono);
    font-size: 12.5px;
    user-select: none;
    outline: none;
  }
  .grid:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .header {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    height: 34px;
    background: var(--panel-2);
    border-bottom: 1px solid var(--border);
  }
  .body {
    position: relative;
  }
  .row {
    position: absolute;
    left: 0;
    display: flex;
    height: 26px;
  }
  .row.stripe {
    background: var(--stripe);
  }
  .row:hover {
    background: var(--hover);
  }
  .cell {
    flex: none;
    height: 100%;
    padding: 0 10px;
    line-height: 26px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border-right: 1px solid var(--border);
  }
  .cell.numeric {
    text-align: right;
  }
  .cell.num {
    position: sticky;
    left: 0;
    z-index: 1;
    padding: 0 8px;
    text-align: right;
    color: var(--muted);
    background: var(--panel-2);
  }
  .header .cell.num {
    z-index: 3;
  }
  .cell.sel {
    background: var(--selected);
  }
  .cell.focus {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .cell.num {
    cursor: pointer;
  }
  .cell.num.rowsel {
    background: var(--selected);
    color: var(--text);
  }
  .null {
    color: var(--null);
    font-style: italic;
  }
  .hcell {
    position: relative;
    flex: none;
    height: 34px;
    border-right: 1px solid var(--border);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    width: 100%;
    height: 34px;
    padding: 0 10px;
    border: 0;
    border-radius: 0;
    background: transparent;
    font-family: var(--ui);
    text-align: left;
  }
  .resizer {
    position: absolute;
    top: 0;
    right: -4px;
    z-index: 2;
    width: 9px;
    height: 100%;
    cursor: col-resize;
    touch-action: none;
  }
  .resizer:hover,
  .resizer:active {
    background: linear-gradient(to right, transparent 3px, var(--accent) 3px, var(--accent) 5px, transparent 5px);
  }
  .head.numeric {
    flex-direction: row-reverse;
  }
  .label {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.2;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .type {
    color: var(--muted);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .arrow {
    color: var(--accent);
    font-size: 9px;
  }

  .toast {
    position: absolute;
    right: 16px;
    bottom: 14px;
    padding: 6px 12px;
    border-radius: 8px;
    background: var(--text);
    color: var(--panel);
    font-family: var(--ui);
    font-size: 12px;
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.25);
    pointer-events: none;
  }
  .ctxmenu {
    position: fixed;
    z-index: 50;
    display: flex;
    flex-direction: column;
    min-width: 180px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--panel);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.2);
    font-family: var(--ui);
  }
  .ctxmenu button {
    display: flex;
    justify-content: space-between;
    gap: 24px;
    border: 0;
    background: transparent;
    text-align: left;
  }
  .ctxmenu kbd {
    color: var(--muted);
    font: inherit;
    font-size: 11px;
  }
</style>
