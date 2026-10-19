<script lang="ts">
  import { onMount } from 'svelte';

  export interface MenuItem {
    label: string;
    run: () => void;
    hint?: string;
    separatorBefore?: boolean;
  }

  let {
    x,
    y,
    items,
    onclose,
  }: { x: number; y: number; items: MenuItem[]; onclose: () => void } = $props();

  let el: HTMLDivElement;
  let left = $state(0);
  let top = $state(0);

  // Keep the menu inside the window.
  onMount(() => {
    const r = el.getBoundingClientRect();
    left = Math.max(4, Math.min(x, window.innerWidth - r.width - 4));
    top = Math.max(4, Math.min(y, window.innerHeight - r.height - 4));
    el.querySelector<HTMLButtonElement>('button')?.focus();
  });

  function pick(item: MenuItem) {
    onclose();
    item.run();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onclose();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const buttons = [...el.querySelectorAll<HTMLButtonElement>('button')];
      const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
      const next = (i + (e.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
      buttons[next].focus();
    }
  }
</script>

<svelte:window
  onmousedown={(e) => {
    if (!el.contains(e.target as Node)) onclose();
  }}
  onblur={onclose}
  {onkeydown}
/>

<div class="menu" bind:this={el} style:left="{left}px" style:top="{top}px" role="menu" data-testid="context-menu">
  {#each items as item (item.label)}
    {#if item.separatorBefore}<div class="sep" role="separator"></div>{/if}
    <button role="menuitem" onclick={() => pick(item)}>
      <span>{item.label}</span>
      {#if item.hint}<kbd>{item.hint}</kbd>{/if}
    </button>
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 60;
    display: flex;
    flex-direction: column;
    min-width: 200px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--panel);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.2);
  }
  button {
    display: flex;
    justify-content: space-between;
    gap: 24px;
    border: 0;
    background: transparent;
    text-align: left;
  }
  kbd {
    color: var(--muted);
    font: inherit;
    font-size: 11px;
  }
  .sep {
    height: 1px;
    margin: 4px 2px;
    background: var(--border);
  }
</style>
