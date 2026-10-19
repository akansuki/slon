<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import { durationLabel, preview, whenLabel } from './history';
  import Icon from './Icon.svelte';
  import { app } from './store.svelte';
  import type { HistoryEntry } from './types';

  let search = $state('');
  let allServers = $state(false);
  let entries = $state.raw<HistoryEntry[]>([]);
  let loaded = $state(false);
  let confirmClear = $state(false);
  let confirmTimer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  let searchEl: HTMLInputElement;

  async function load() {
    const mine = ++seq;
    try {
      const list = await api.historyList(search.trim() || null, allServers ? null : app.serverKey);
      if (mine === seq) entries = list;
    } catch {
      if (mine === seq) entries = [];
    } finally {
      if (mine === seq) loaded = true;
    }
  }

  // Reload when the filters change (typing is debounced) or a statement was just recorded.
  let debounce: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    void search;
    void allServers;
    void app.historyVersion;
    clearTimeout(debounce);
    debounce = setTimeout(load, 120);
    return () => clearTimeout(debounce);
  });

  onMount(() => searchEl.focus());

  function database(e: HistoryEntry) {
    return e.server === app.serverKey ? e.database : undefined;
  }

  function inNewTab(e: HistoryEntry) {
    app.newTab(database(e), e.sql);
    app.historyOpen = false;
  }

  function inThisTab(e: HistoryEntry) {
    const tab = app.activeTab;
    if (!tab) return inNewTab(e);
    tab.sql = e.sql;
    app.historyOpen = false;
  }

  async function remove(e: HistoryEntry) {
    try {
      await api.historyDelete(e.id);
    } catch {
      /* shown again by the reload */
    }
    await load();
  }

  async function clear() {
    if (!confirmClear) {
      confirmClear = true;
      clearTimeout(confirmTimer);
      confirmTimer = setTimeout(() => (confirmClear = false), 3500);
      return;
    }
    confirmClear = false;
    try {
      await api.historyClear(allServers ? null : app.serverKey);
    } catch {
      /* the reload shows what is left */
    }
    await load();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      app.historyOpen = false;
    }
  }
</script>

<svelte:window {onkeydown} />

<aside class="panel" aria-label="Query history" data-testid="history-panel">
  <div class="head">
    <strong>History</strong>
    <span class="spacer"></span>
    <button class="ghost" title="Close (Esc)" onclick={() => (app.historyOpen = false)} data-testid="history-close">
      <Icon name="close" size={14} />
    </button>
  </div>

  <div class="filters">
    <input
      bind:this={searchEl}
      type="search"
      placeholder="Search statements"
      bind:value={search}
      data-testid="history-search"
    />
    <label class="check">
      <input type="checkbox" bind:checked={allServers} data-testid="history-all" />
      <span>All servers</span>
    </label>
  </div>

  <div class="list" data-testid="history-list">
    {#each entries as e (e.id)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="entry" class:failed={!e.ok} ondblclick={() => inNewTab(e)} data-testid="history-entry">
        <pre class="sql">{preview(e.sql)}</pre>
        <div class="meta">
          <span class="status" title={e.ok ? 'Succeeded' : (e.error ?? 'Failed')}>{e.ok ? '✓' : '✗'}</span>
          <span>{whenLabel(e.at)}</span>
          <span>{durationLabel(e.durationMs)}</span>
          {#if e.rows !== null}<span>{e.rows.toLocaleString()} row{e.rows === 1 ? '' : 's'}</span>{/if}
          {#if e.count > 1}<span title="Run {e.count} times in a row">×{e.count}</span>{/if}
          <span class="where" title={e.server}>{allServers ? `${e.server} · ` : ''}{e.database}</span>
        </div>
        {#if !e.ok && e.error}<div class="error">{e.error}</div>{/if}
        <div class="actions">
          <button onclick={() => inNewTab(e)} title="Open in a new tab (double-click)" data-testid="history-new-tab">New tab</button>
          <button onclick={() => inThisTab(e)} title="Replace the text of the current tab" data-testid="history-use">Use here</button>
          <span class="spacer"></span>
          <button class="ghost" onclick={() => remove(e)} title="Remove from history" data-testid="history-delete">
            <Icon name="trash" size={13} />
          </button>
        </div>
      </div>
    {:else}
      <p class="empty" data-testid="history-empty">
        {#if !loaded}
          Loading…
        {:else if search.trim()}
          Nothing matches “{search.trim()}”.
        {:else if !app.historyEnabled}
          History is switched off. Statements you run are not recorded.
        {:else}
          Statements you run will show up here.
        {/if}
      </p>
    {/each}
  </div>

  <div class="foot">
    <label class="check" title="When off, nothing new is written to the history file">
      <input
        type="checkbox"
        checked={app.historyEnabled}
        onchange={(e) => app.setHistoryEnabled(e.currentTarget.checked)}
        data-testid="history-record"
      />
      <span>Record history</span>
    </label>
    <span class="spacer"></span>
    <button class:danger={confirmClear} onclick={clear} disabled={entries.length === 0} data-testid="history-clear">
      {confirmClear ? 'Click again to clear' : allServers ? 'Clear all' : 'Clear this server'}
    </button>
  </div>
</aside>

<style>
  .panel {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 60;
    width: min(440px, 92vw);
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--border);
    background: var(--panel);
    box-shadow: -6px 0 24px rgb(0 0 0 / 0.18);
  }
  .head,
  .filters,
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
  }
  .head {
    height: 40px;
    border-bottom: 1px solid var(--border);
  }
  .filters {
    border-bottom: 1px solid var(--border);
  }
  .filters input[type='search'] {
    flex: 1;
    min-width: 0;
  }
  .foot {
    border-top: 1px solid var(--border);
  }
  .spacer {
    flex: 1;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 5px;
    flex: none;
    white-space: nowrap;
    color: var(--muted);
    font-size: 12px;
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 6px 8px;
  }
  .entry {
    padding: 8px 10px;
    margin-bottom: 6px;
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .entry:hover {
    background: var(--hover);
  }
  .sql {
    margin: 0;
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
    margin-top: 5px;
    color: var(--muted);
    font-size: 11px;
  }
  .status {
    color: #2e7d32;
    font-weight: 700;
  }
  .failed .status {
    color: var(--danger);
  }
  .where {
    margin-left: auto;
    max-width: 55%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error {
    margin-top: 4px;
    color: var(--danger);
    font-size: 11px;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 6px;
  }
  .empty {
    margin: 24px 12px;
    color: var(--muted);
    text-align: center;
  }
  .danger {
    background: var(--danger-bg);
    color: var(--danger);
  }
</style>
