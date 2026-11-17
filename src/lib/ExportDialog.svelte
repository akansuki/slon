<script lang="ts">
  import { onMount } from 'svelte';
  import { api, toPgError } from './api';
  import {
    CHUNK_ROWS,
    extensionFor,
    labelFor,
    loadOptions,
    rememberOptions,
    suggestName,
    type ExportOptions,
  } from './export';
  import { sortedOrder, type SortSpec } from './sort';
  import type { Cell, Column } from './types';

  let {
    scope,
    columns,
    rows,
    sort,
    truncated,
    onclose,
    ondone,
    onall,
  }: {
    /**
     * `grid` saves the rows the grid holds, in the order on screen. `all` hands the chosen
     * options back so the caller can re-run the statement and stream every row to the file;
     * sorting is not offered there, because rows are written as they arrive.
     */
    scope: 'grid' | 'all';
    columns: Column[];
    rows: Cell[][];
    sort: SortSpec | null;
    truncated: boolean;
    onclose: () => void;
    ondone: (message: string) => void;
    onall: (options: ExportOptions) => Promise<boolean>;
  } = $props();

  let options = $state<ExportOptions>(loadOptions());
  let busy = $state(false);
  let error = $state<string | null>(null);
  let written = $state(0);
  let dialogEl: HTMLDivElement;

  const total = $derived(rows.length);
  const ext = $derived(extensionFor(options));

  onMount(() => dialogEl.querySelector<HTMLElement>('select, button')?.focus());

  async function save() {
    if (busy) return;
    error = null;
    busy = true;
    written = 0;
    try {
      const o = $state.snapshot(options);
      rememberOptions(o);
      if (scope === 'all') {
        // The run owns the file from here: it picks the path, executes once, and streams
        // every row out. Closing only when something was actually started keeps the dialog
        // up if the save dialog was dismissed.
        if (await onall(o)) onclose();
        return;
      }
      const path = await api.chooseExportPath(suggestName(o), labelFor(o), [extensionFor(o)]);
      if (path === null) return; // dialog cancelled, stay open
      // The order on screen: rows are never moved in the grid, so apply the sort here.
      let source: readonly Cell[][] = rows;
      if (sort) {
        const order = sortedOrder(rows, sort.col, columns[sort.col]?.sortKind ?? 'unknown', sort.dir);
        source = Array.from(order, (i) => rows[i]);
      }
      // An empty result still has to produce a well-formed file, so write once either way.
      for (let from = 0; from === 0 || from < source.length; from += CHUNK_ROWS) {
        const to = Math.min(from + CHUNK_ROWS, source.length);
        await api.writeExportRows(path, columns, source.slice(from, to) as Cell[][], o, from, to >= source.length);
        written = to;
      }
      ondone(`Saved ${source.length.toLocaleString()} row${source.length === 1 ? '' : 's'} to ${path}`);
      onclose();
    } catch (e) {
      error = toPgError(e).message;
    } finally {
      busy = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && !busy) {
      e.preventDefault();
      onclose();
    }
  }
</script>

<svelte:window {onkeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={() => !busy && onclose()}>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-label={scope === 'all' ? 'Export all rows' : 'Export results'}
    tabindex="-1"
    bind:this={dialogEl}
    onclick={(e) => e.stopPropagation()}
    data-testid="export-dialog"
  >
    <h2>{scope === 'all' ? 'Export all rows' : 'Export results'}</h2>
    {#if scope === 'all'}
      <p class="what" data-testid="export-summary">
        The statement is run and every row it returns is written straight to the file, so
        the grid's 100,000-row limit does not apply.
      </p>
      <p class="hint">
        Rows are written as they arrive, in the order the server sends them. For a particular
        order, put an <code>ORDER BY</code> in the statement.
      </p>
    {:else}
      <p class="what" data-testid="export-summary">
        {total.toLocaleString()} row{total === 1 ? '' : 's'}, {columns.length} column{columns.length === 1 ? '' : 's'}
        {sort ? ', in the sorted order shown' : ''}
      </p>
      {#if truncated}
        <p class="warn" data-testid="export-truncated">
          The grid holds only the first {total.toLocaleString()} rows of the result, so that is all that will be
          saved. Use <b>Export all rows</b> to write the whole result.
        </p>
      {/if}
    {/if}

    <label>
      <span>Format</span>
      <select bind:value={options.format} disabled={busy} data-testid="export-format">
        <option value="csv">CSV / delimited text</option>
        <option value="json">JSON (array of objects)</option>
        <option value="jsonl">JSON Lines (one object per line)</option>
      </select>
    </label>

    {#if options.format === 'csv'}
      <label>
        <span>Delimiter</span>
        <select bind:value={options.delimiter} disabled={busy} data-testid="export-delimiter">
          <option value=",">Comma (,)</option>
          <option value=";">Semicolon (;)</option>
          <option value={'\t'}>Tab</option>
        </select>
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={options.header} disabled={busy} data-testid="export-header" />
        <span>First line holds the column names</span>
      </label>
      <label class="check" title="Excel on Windows needs it to show č, ć, š, đ and other non-ASCII letters correctly">
        <input type="checkbox" bind:checked={options.bom} disabled={busy} data-testid="export-bom" />
        <span>Add a UTF-8 byte order mark (for Excel)</span>
      </label>
      <p class="hint">NULL is written as an empty field, an empty string as <code>""</code>.</p>
    {:else}
      <p class="hint">Numbers keep the digits the server sent, json/jsonb columns are embedded as JSON, everything else is text.</p>
    {/if}

    {#if error}<p class="error" role="alert" data-testid="export-error">{error}</p>{/if}

    <div class="actions">
      {#if busy && written > 0 && scope === 'grid'}
        <span class="progress">{written.toLocaleString()} / {total.toLocaleString()}</span>
      {/if}
      <span class="spacer"></span>
      <button onclick={onclose} disabled={busy} data-testid="export-cancel">Cancel</button>
      <button class="primary" onclick={save} disabled={busy} data-testid="export-save">
        {busy ? 'Saving…' : `Save as .${ext}…`}
      </button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.35);
  }
  .dialog {
    width: min(440px, calc(100vw - 32px));
    padding: 16px 18px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--panel);
    box-shadow: 0 10px 40px rgb(0 0 0 / 0.3);
    display: grid;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
  }
  .what,
  .hint {
    margin: 0;
    color: var(--muted);
    font-size: 12px;
  }
  .warn {
    margin: 0;
    padding: 6px 8px;
    border-radius: 6px;
    background: var(--warn-bg);
    color: var(--warn);
    font-size: 12px;
  }
  .error {
    margin: 0;
    padding: 6px 8px;
    border-radius: 6px;
    background: var(--danger-bg);
    color: var(--danger);
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  label {
    display: grid;
    grid-template-columns: 90px 1fr;
    align-items: center;
    gap: 8px;
  }
  label.check {
    grid-template-columns: auto 1fr;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .spacer {
    flex: 1;
  }
  .progress {
    color: var(--muted);
    font-size: 12px;
  }
  code {
    font-family: var(--mono);
  }
</style>
