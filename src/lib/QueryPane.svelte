<script lang="ts">
  import Editor from './Editor.svelte';
  import ExportDialog from './ExportDialog.svelte';
  import Grid from './Grid.svelte';
  import Icon from './Icon.svelte';
  import MessagesView from './MessagesView.svelte';
  import { hasWarning } from './messages';
  import PlanView from './PlanView.svelte';
  import { app, type QueryTab } from './store.svelte';

  let { tab }: { tab: QueryTab } = $props();

  let editor: Editor;
  let pane: HTMLDivElement;
  let editorHeight = $state(260);

  const running = $derived(tab.status === 'running');
  let exporting = $state(false);
  const canExport = $derived(tab.columns.length > 0 && tab.status === 'idle' && tab.rows.length > 0);

  function run() {
    if (tab.status !== 'idle') return;
    const { text, offset } = editor.getRunText();
    editor.clearError();
    void tab.run(text, offset);
  }

  function explain(analyze: boolean) {
    if (tab.status !== 'idle') return;
    const { text, offset } = editor.getRunText();
    editor.clearError();
    void tab.explain(text, offset, analyze);
  }

  const messageCount = $derived.by(() => {
    void tab.messagesVersion;
    return tab.messages.length;
  });
  const messageWarning = $derived.by(() => {
    void tab.messagesVersion;
    return hasWarning(tab.messages);
  });

  // The text can be replaced from outside (picking a statement from the history).
  $effect(() => {
    const text = tab.sql;
    if (editor && editor.getText() !== text) editor.setText(text);
  });

  // Underline the position the server pointed at.
  $effect(() => {
    void tab.errorSeq;
    const e = tab.error;
    if (e?.kind === 'query' && e.position) editor?.showError(e.position, tab.errorOffset, e.message);
  });

  function startDrag(e: PointerEvent) {
    const startY = e.clientY;
    const startH = editorHeight;
    const move = (ev: PointerEvent) => {
      editorHeight = Math.max(80, Math.min(startH + ev.clientY - startY, pane.clientHeight - 170));
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  function duration(ms: number | null) {
    if (ms === null) return '';
    return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(2)} s`;
  }

  const rowCount = $derived.by(() => {
    void tab.rowsVersion;
    return tab.rows.length;
  });
</script>

{#if exporting}
  <ExportDialog
    columns={tab.columns}
    rows={tab.rows}
    sort={tab.sort}
    truncated={tab.truncated}
    onclose={() => (exporting = false)}
    ondone={(m) => app.flash(m)}
  />
{/if}

<div class="pane" bind:this={pane}>
  <div class="toolbar">
    <button class="primary" onclick={run} disabled={tab.status !== 'idle'} title="Run (Ctrl+Enter or F5). Runs the selection if there is one.">
      <Icon name="play" size={14} /> Run
    </button>
    <button onclick={() => tab.cancel()} disabled={!running} title="Cancel the running query">
      <Icon name="stop" size={14} /> Cancel
    </button>
    <button
      onclick={() => explain(false)}
      disabled={tab.status !== 'idle'}
      title="Show the execution plan the server would use. The statement is not run."
      data-testid="explain"
    >
      <Icon name="plan" size={14} /> Explain
    </button>
    <button
      onclick={() => explain(true)}
      disabled={tab.status !== 'idle'}
      title="Run the statement and show the real plan with timings. Changes are rolled back."
      data-testid="explain-analyze"
    >
      <Icon name="plan" size={14} /> Explain analyze
    </button>
    <button
      onclick={() => (exporting = true)}
      disabled={!canExport}
      title="Save the rows in the grid as CSV or JSON"
      data-testid="export"
    >
      <Icon name="export" size={14} /> Export
    </button>
    <button
      class="ghost"
      onclick={() => tab.loadCompletion()}
      disabled={tab.completionLoading}
      title="Reload table and column names used for autocompletion"
      data-testid="refresh-schema"
    >
      <Icon name="refresh" size={14} /> Refresh names
    </button>
    <span class="spacer"></span>
    <span class="db" title="Database of this tab"><Icon name="database" size={14} /> {tab.database}</span>
  </div>

  <div class="editor" style:height="{editorHeight}px">
    <Editor bind:this={editor} value={tab.sql} schema={tab.completion} onchange={(v) => (tab.sql = v)} onrun={run} />
  </div>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="splitter" onpointerdown={startDrag} role="separator" aria-orientation="horizontal" aria-label="Resize editor"></div>

  <div class="results">
    {#if tab.error}
      <div class="banner" class:cancelled={tab.error.kind === 'cancelled'} role="alert">
        <Icon name="alert" size={16} />
        <div class="text">
          <strong>{tab.error.message}</strong>
          {#if tab.error.detail}<div>{tab.error.detail}</div>{/if}
          {#if tab.error.hint}<div class="meta">Hint: {tab.error.hint}</div>{/if}
          {#if tab.error.sqlstate && tab.error.kind !== 'cancelled'}
            <div class="meta">SQLSTATE {tab.error.sqlstate}</div>
          {/if}
        </div>
        {#if tab.error.kind === 'connection'}
          <button onclick={() => tab.reconnect()}>Reconnect</button>
        {/if}
      </div>
    {/if}

    {#if tab.plan || messageCount > 0}
      <div class="views" role="tablist" aria-label="Result view">
        <button role="tab" aria-selected={tab.view === 'results'} class:active={tab.view === 'results'} onclick={() => (tab.view = 'results')} data-testid="view-results">
          Results
        </button>
        {#if messageCount > 0}
          <button role="tab" aria-selected={tab.view === 'messages'} class:active={tab.view === 'messages'} onclick={() => (tab.view = 'messages')} data-testid="view-messages">
            Messages <span class="badge" class:warn={messageWarning} data-testid="messages-badge">{messageCount.toLocaleString()}</span>
          </button>
        {/if}
        {#if tab.plan}
          <button role="tab" aria-selected={tab.view === 'plan'} class:active={tab.view === 'plan'} onclick={() => (tab.view = 'plan')} data-testid="view-plan">
            Plan{tab.plan.analyzed ? '' : ' (estimated)'}
          </button>
        {/if}
      </div>
    {/if}

    <div class="body">
      {#if tab.view === 'messages'}
        <MessagesView messages={tab.messages} version={tab.messagesVersion} {running} />
      {:else if tab.view === 'plan' && tab.plan}
        <PlanView plan={tab.plan} json={tab.planJson} sql={tab.planSql} />
      {:else if tab.columns.length > 0}
        <Grid columns={tab.columns} rows={tab.rows} version={tab.rowsVersion} {running} bind:sort={tab.sort} />
      {:else if tab.message}
        <div class="placeholder">{tab.message}</div>
      {:else if !tab.error}
        <div class="placeholder">
          {tab.status === 'opening' ? 'Connecting…' : running ? 'Running…' : 'Run a query to see its results here.'}
        </div>
      {/if}
    </div>
  </div>

  <div class="status">
    {#if running}
      <span class="spinner"></span>
      <span>Running… {rowCount.toLocaleString()} rows received</span>
    {:else if tab.view === 'messages'}
      <span>{messageCount.toLocaleString()} message{messageCount === 1 ? '' : 's'}</span>
      {#if tab.message}
        <span class="sep">·</span>
        <span class="statusmsg">{tab.message}</span>
      {/if}
      <span class="sep">·</span>
      <span>{duration(tab.elapsedMs)}</span>
    {:else if tab.view === 'plan' && tab.plan}
      <span>{tab.plan.analyzed ? 'Plan with actual timings' : 'Estimated plan'}</span>
      <span class="sep">·</span>
      <span>{duration(tab.elapsedMs)}</span>
    {:else if tab.columns.length > 0}
      <span>{rowCount.toLocaleString()} row{rowCount === 1 ? '' : 's'}</span>
      <span class="sep">·</span>
      <span>{duration(tab.elapsedMs)}</span>
      {#if tab.truncated}
        <span class="warn">
          <Icon name="alert" size={13} />
          <span class="warntext">
            Showing only the first {rowCount.toLocaleString()} rows; sorting applies to these rows only
          </span>
        </span>
      {/if}
    {:else if tab.elapsedMs !== null}
      <span>{duration(tab.elapsedMs)}</span>
    {/if}
    <span class="spacer"></span>
    <span class="hintkeys">Ctrl+Enter runs · a selection runs alone</span>
  </div>
</div>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
  }
  .spacer {
    flex: 1;
  }
  .db {
    display: inline-flex;
    gap: 6px;
    align-items: center;
    color: var(--muted);
  }
  .editor {
    flex: none;
    min-height: 0;
    border-bottom: 1px solid var(--border);
  }
  .splitter {
    flex: none;
    height: 5px;
    margin: -3px 0;
    cursor: row-resize;
    position: relative;
    z-index: 5;
  }
  .splitter:hover {
    background: var(--accent);
    opacity: 0.5;
  }
  .results {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .body {
    flex: 1;
    min-height: 0;
  }
  .views {
    display: flex;
    gap: 2px;
    padding: 4px 10px 0;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
  }
  .views button {
    border-radius: 6px 6px 0 0;
    border-bottom-color: transparent;
    background: transparent;
    color: var(--muted);
  }
  .badge {
    display: inline-block;
    min-width: 18px;
    padding: 0 5px;
    margin-left: 2px;
    border-radius: 9px;
    background: var(--panel-2);
    border: 1px solid var(--border);
    font-size: 11px;
    text-align: center;
  }
  .badge.warn {
    background: var(--warn-bg);
    color: var(--warn);
    border-color: var(--warn);
  }
  .statusmsg {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .views button.active {
    background: var(--panel-2);
    color: var(--text);
    font-weight: 600;
  }
  .banner {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 10px 14px;
    background: var(--danger-bg);
    color: var(--danger);
    border-bottom: 1px solid var(--border);
    max-height: 40%;
    overflow: auto;
  }
  .banner.cancelled {
    background: var(--warn-bg);
    color: var(--warn);
  }
  .banner .text {
    flex: 1;
    word-break: break-word;
    white-space: pre-wrap;
  }
  .meta {
    margin-top: 2px;
    font-size: 12px;
    opacity: 0.85;
  }
  .placeholder {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--muted);
    padding: 24px;
    text-align: center;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 28px;
    padding: 0 12px;
    border-top: 1px solid var(--border);
    background: var(--panel-2);
    color: var(--muted);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
  }
  .sep {
    opacity: 0.5;
  }
  .warn {
    display: inline-flex;
    flex: 0 1 auto;
    min-width: 0;
    gap: 5px;
    align-items: center;
    margin-left: 10px;
    color: var(--warn);
  }
  .warntext {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .hintkeys {
    flex: none;
    opacity: 0.8;
  }
</style>
