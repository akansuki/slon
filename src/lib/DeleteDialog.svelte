<script lang="ts">
  import { onMount } from 'svelte';
  import { toPgError } from './api';
  import Icon from './Icon.svelte';
  import { rowsLabel } from './rowdelete';
  import type { QueryTab } from './store.svelte';
  import type { DeletePlan } from './types';

  let {
    tab,
    onclose,
    ondone,
  }: { tab: QueryTab; onclose: () => void; ondone: (message: string) => void } = $props();

  let plan = $state.raw<DeletePlan | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let cancelEl: HTMLButtonElement;

  // The statement is built by the server, from the same input the delete will use, so what
  // is shown here is what runs.
  onMount(async () => {
    cancelEl?.focus();
    try {
      plan = await tab.planDelete();
    } catch (e) {
      error = toPgError(e).message;
    }
  });

  async function confirm() {
    if (busy || !plan) return;
    busy = true;
    error = null;
    try {
      const out = await tab.deleteSelected();
      ondone(`Deleted ${rowsLabel(out.deleted)} in ${out.elapsedMs} ms`);
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
    role="alertdialog"
    aria-modal="true"
    aria-label="Delete rows"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    data-testid="delete-dialog"
  >
    <h2><Icon name="trash" size={15} /> Delete {rowsLabel(tab.selectedRows.length)}?</h2>

    {#if plan}
      <p class="what" data-testid="delete-summary">
        From <strong>{plan.identity.schema}.{plan.identity.table}</strong>, matched on
        {plan.identity.key.map((k) => k.name).join(', ')}.
      </p>
      <pre class="sql" data-testid="delete-sql">{plan.sql}</pre>
      <p class="hint">
        This runs in a transaction and is rolled back unless exactly {rowsLabel(plan.rowCount)}
        {plan.rowCount === 1 ? 'is' : 'are'} deleted. It cannot be undone afterwards.
      </p>
    {:else if !error}
      <p class="what">Working out the statement…</p>
    {/if}

    {#if error}<p class="error" role="alert" data-testid="delete-error">{error}</p>{/if}

    <div class="actions">
      <span class="spacer"></span>
      <button bind:this={cancelEl} onclick={onclose} disabled={busy} data-testid="delete-cancel">
        Cancel
      </button>
      <button
        class="danger"
        onclick={confirm}
        disabled={busy || !plan}
        data-testid="delete-confirm"
      >
        {busy ? 'Deleting…' : 'Delete'}
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
    width: min(560px, calc(100vw - 32px));
    padding: 16px 18px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--panel);
    box-shadow: 0 10px 40px rgb(0 0 0 / 0.3);
    display: grid;
    gap: 10px;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 7px;
    margin: 0;
    font-size: 15px;
  }
  .what,
  .hint {
    margin: 0;
    color: var(--muted);
    font-size: 12px;
  }
  .sql {
    margin: 0;
    max-height: 180px;
    overflow: auto;
    padding: 9px 11px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
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
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .spacer {
    flex: 1;
  }
  .danger {
    background: var(--danger);
    border-color: var(--danger);
    color: #fff;
  }
  .danger:disabled {
    opacity: 0.6;
  }
</style>
