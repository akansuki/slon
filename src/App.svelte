<script lang="ts">
  import ConnectForm from './lib/ConnectForm.svelte';
  import HistoryPanel from './lib/HistoryPanel.svelte';
  import Icon from './lib/Icon.svelte';
  import QueryPane from './lib/QueryPane.svelte';
  import TreeItem from './lib/TreeItem.svelte';
  import { clampSidebar, parseStoredSidebar, SIDEBAR_DEFAULT, SIDEBAR_KEY, SIDEBAR_MIN, sidebarMax } from './lib/layout';
  import { onMount } from 'svelte';
  import { serverLabel, versionLabel } from './lib/about';
  import { app } from './lib/store.svelte';

  // Remembered between runs. Storage can be unavailable (private mode, blocked), so every access
  // is guarded and the app simply starts with the default width.
  function storedWidth(): number {
    try {
      return parseStoredSidebar(localStorage.getItem(SIDEBAR_KEY)) ?? SIDEBAR_DEFAULT;
    } catch {
      return SIDEBAR_DEFAULT;
    }
  }
  function storeWidth(w: number) {
    try {
      localStorage.setItem(SIDEBAR_KEY, String(w));
    } catch {
      /* not remembered, still works */
    }
  }

  let windowWidth = $state(window.innerWidth);
  let wanted = $state(storedWidth());
  let dragging = $state(false);
  // The window can be made narrower than the remembered width allows.
  const sidebarWidth = $derived(clampSidebar(wanted, windowWidth));

  function setWidth(w: number) {
    wanted = clampSidebar(w, windowWidth);
  }

  // Mouse events rather than pointer events: WebView2 handles both, but pointer capture
  // proved unreliable in WebKitGTK (the end-to-end test environment).
  function startResize(e: MouseEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    const startX = e.clientX;
    const startW = sidebarWidth;
    dragging = true;
    const move = (ev: MouseEvent) => setWidth(startW + ev.clientX - startX);
    const up = () => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
      dragging = false;
      storeWidth(wanted);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  }

  function resizeKey(e: KeyboardEvent) {
    const step = e.shiftKey ? 64 : 16;
    let next: number | null = null;
    if (e.key === 'ArrowLeft') next = sidebarWidth - step;
    else if (e.key === 'ArrowRight') next = sidebarWidth + step;
    else if (e.key === 'Home') next = SIDEBAR_MIN;
    else if (e.key === 'End') next = sidebarMax(windowWidth);
    else if (e.key === 'Enter') next = SIDEBAR_DEFAULT;
    if (next === null) return;
    e.preventDefault();
    setWidth(next);
    storeWidth(wanted);
  }

  function resetWidth() {
    setWidth(SIDEBAR_DEFAULT);
    storeWidth(wanted);
  }

  onMount(() => void app.loadBuildInfo());

  const buildLabel = $derived(versionLabel(app.build));
  const pgLabel = $derived(serverLabel(app.connection?.serverVersion));

  // Remember the open tabs (debounced) whenever their text, names, number or order change.
  $effect(() => {
    if (!app.connection) return;
    for (const t of app.tabs) {
      void t.sql;
      void t.title;
    }
    void app.activeKey;
    app.scheduleSave();
  });

  // A restored tab loads its autocompletion names the first time it is shown.
  $effect(() => {
    app.activeTab?.ensureCompletion();
  });

  function flushOnHide() {
    if (document.visibilityState === 'hidden') void app.flushSave();
  }

  function onkeydown(e: KeyboardEvent) {
    if (!app.connection || !(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey) return;
    if (e.key === 't') {
      e.preventDefault();
      app.newTab();
    } else if (e.key === 'h') {
      e.preventDefault();
      app.historyOpen = !app.historyOpen;
    } else if (e.key === 'w' && app.activeKey !== null) {
      e.preventDefault();
      void app.closeTab(app.activeKey);
    }
  }
</script>

<svelte:window {onkeydown} bind:innerWidth={windowWidth} />
<svelte:document onvisibilitychange={flushOnHide} />

{#if !app.connection}
  <ConnectForm />
{:else}
  <div class="shell" class:dragging style:--sidebar-width="{sidebarWidth}px">
    <aside class="sidebar">
      <div class="head">
        <span>Servers</span>
        <button class="ghost" title="Disconnect" onclick={() => app.disconnect()}>
          <Icon name="plug" size={14} /> Disconnect
        </button>
      </div>
      <div class="tree" role="tree">
        {#if app.root}<TreeItem node={app.root} />{/if}
      </div>
      <button
        class="foot"
        title="Click to copy version details for a bug report"
        onclick={() => app.copyDiagnostics()}
        data-testid="about"
      >
        {buildLabel}{pgLabel ? ` · ${pgLabel}` : ''}
      </button>
    </aside>

    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="splitter"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize sidebar"
      aria-valuemin={SIDEBAR_MIN}
      aria-valuemax={sidebarMax(windowWidth)}
      aria-valuenow={sidebarWidth}
      tabindex="0"
      title="Drag to resize, double-click to reset"
      data-testid="sidebar-splitter"
      onmousedown={startResize}
      ondblclick={resetWidth}
      onkeydown={resizeKey}
    ></div>

    <main>
      <div class="tabs" role="tablist">
        {#each app.tabs as t (t.key)}
          <div
            class="tab"
            class:active={t.key === app.activeKey}
            role="tab"
            aria-selected={t.key === app.activeKey}
            tabindex="0"
            onclick={() => (app.activeKey = t.key)}
            onauxclick={(e) => e.button === 1 && app.closeTab(t.key)}
            onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (app.activeKey = t.key)}
          >
            {#if t.status === 'running'}<span class="spinner"></span>{/if}
            <span class="title">{t.title}</span>
            <button
              class="ghost close"
              title="Close tab (Ctrl+W)"
              onclick={(e) => {
                e.stopPropagation();
                void app.closeTab(t.key);
              }}
            >
              <Icon name="close" size={12} />
            </button>
          </div>
        {/each}
        <button class="ghost add" title="New query tab (Ctrl+T)" onclick={() => app.newTab()}>
          <Icon name="plus" size={14} />
        </button>
        <span class="grow"></span>
        <button
          class="ghost historybtn"
          class:on={app.historyOpen}
          title="Query history (Ctrl+H)"
          onclick={() => (app.historyOpen = !app.historyOpen)}
          data-testid="history-open"
        >
          <Icon name="history" size={14} /> History
        </button>
      </div>

      <div class="bodies">
        {#each app.tabs as t (t.key)}
          <div class="body" hidden={t.key !== app.activeKey}>
            <QueryPane tab={t} />
          </div>
        {/each}
        {#if app.tabs.length === 0}
          <div class="empty">
            <p>No query tabs open.</p>
            <button class="primary" onclick={() => app.newTab()}><Icon name="plus" size={14} /> New query tab</button>
          </div>
        {/if}
      </div>
    </main>
    {#if app.historyOpen}<HistoryPanel />{/if}
    {#if app.notice}<div class="notice" role="status" data-testid="notice">{app.notice}</div>{/if}
  </div>
{/if}

<style>
  .notice {
    position: fixed;
    right: 16px;
    bottom: 40px;
    z-index: 70;
    max-width: 420px;
    padding: 6px 12px;
    border-radius: 8px;
    background: var(--text);
    color: var(--panel);
    font-size: 12px;
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.25);
    pointer-events: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-width, 290px) 1fr;
    height: 100%;
    position: relative;
  }
  .shell.dragging {
    cursor: col-resize;
    user-select: none;
  }
  /* A narrow grab area on the border between sidebar and query area. */
  .splitter {
    position: absolute;
    top: 0;
    bottom: 0;
    left: calc(var(--sidebar-width) - 2px);
    width: 6px;
    z-index: 20;
    cursor: col-resize;
  }
  .splitter:hover,
  .splitter:focus-visible,
  .shell.dragging .splitter {
    background: var(--accent);
    opacity: 0.55;
    outline: none;
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--border);
    background: var(--panel);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 40px;
    padding: 0 8px 0 12px;
    border-bottom: 1px solid var(--border);
    color: var(--muted);
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    font-size: 11px;
  }
  .head button {
    text-transform: none;
    letter-spacing: 0;
    font-weight: 400;
    color: var(--muted);
  }
  .tree {
    flex: 1;
    overflow: auto;
    padding: 6px 0;
  }
  .foot {
    display: block;
    width: 100%;
    height: auto;
    padding: 6px 12px;
    border: 0;
    border-top: 1px solid var(--border);
    border-radius: 0;
    background: none;
    color: var(--muted);
    font-size: 12px;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .foot:hover {
    background: var(--hover);
    color: var(--text);
  }
  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .tabs {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 40px;
    padding: 6px 8px 0;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 220px;
    height: 34px;
    padding: 0 4px 0 12px;
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: 7px 7px 0 0;
    color: var(--muted);
    cursor: default;
    user-select: none;
  }
  .tab:hover {
    background: var(--hover);
  }
  .tab.active {
    background: var(--panel);
    border-color: var(--border);
    margin-bottom: -1px;
    color: var(--text);
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close {
    height: 22px;
    padding: 0 4px;
  }
  .add {
    align-self: center;
    margin-left: 4px;
  }
  .grow {
    flex: 1;
  }
  .historybtn {
    align-self: center;
    color: var(--muted);
  }
  .historybtn.on {
    color: var(--text);
    background: var(--hover);
  }
  .bodies {
    flex: 1;
    min-height: 0;
    position: relative;
    background: var(--panel);
  }
  .body {
    height: 100%;
  }
  .empty {
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 12px;
    height: 100%;
    color: var(--muted);
  }
</style>
