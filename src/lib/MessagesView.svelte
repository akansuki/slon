<script lang="ts">
  import Icon from './Icon.svelte';
  import { fmtOffset, formatAll, toneOf } from './messages';
  import { copyText } from './selection';
  import { app } from './store.svelte';
  import type { ServerMessage } from './types';

  let { messages, version, running }: { messages: ServerMessage[]; version: number; running: boolean } = $props();

  let scroller: HTMLDivElement;
  let stickToEnd = true;

  function onScroll() {
    stickToEnd = scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 24;
  }

  // While messages stream in, keep the newest one in view, unless the person scrolled up.
  $effect(() => {
    void version;
    if (stickToEnd && scroller) queueMicrotask(() => (scroller.scrollTop = scroller.scrollHeight));
  });

  // `messages` is appended to in place, so the list is re-read whenever `version` changes.
  const list = $derived.by(() => {
    void version;
    return messages.slice();
  });
  const count = $derived(list.length);

  async function copyAll() {
    app.flash((await copyText(formatAll(messages))) ? 'Copied messages' : 'Copy failed: clipboard not available');
  }
</script>

<div class="msgs" data-testid="messages-view">
  <div class="bar">
    <span>{count.toLocaleString()} message{count === 1 ? '' : 's'}{running ? ' so far' : ''}</span>
    <span class="spacer"></span>
    <button class="ghost" onclick={copyAll} disabled={count === 0} title="Copy all messages as text" data-testid="messages-copy">
      <Icon name="copy" size={14} /> Copy all
    </button>
  </div>
  <div class="list" bind:this={scroller} onscroll={onScroll}>
    {#if count === 0}
      <div class="empty">{running ? 'Waiting for messages…' : 'The server sent no messages.'}</div>
    {/if}
    {#each list as m, i (i)}
      {@const tone = toneOf(m.severity)}
      <div class="item {tone}" data-testid="message-row">
        <span class="sev" data-testid="message-severity">{m.severity}</span>
        <div class="body">
          <div class="text" data-testid="message-text">{m.message}</div>
          {#if m.detail}<div class="sub"><b>Detail</b> {m.detail}</div>{/if}
          {#if m.hint}<div class="sub"><b>Hint</b> {m.hint}</div>{/if}
          {#if m.context}<div class="sub ctx"><b>Context</b> {m.context}</div>{/if}
        </div>
        <span class="at" title="Time since the run started">{fmtOffset(m.elapsedMs)}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .msgs {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--panel);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--panel-2);
    color: var(--muted);
  }
  .spacer {
    flex: 1;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .empty {
    padding: 24px;
    text-align: center;
    color: var(--muted);
  }
  .item {
    display: grid;
    grid-template-columns: 78px 1fr auto;
    gap: 10px;
    align-items: start;
    padding: 5px 12px;
    border-bottom: 1px solid var(--border);
    /* Thousands of rows stay cheap: the browser skips the ones that are off screen. */
    content-visibility: auto;
    contain-intrinsic-size: auto 28px;
  }
  .sev {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.03em;
    padding: 1px 6px;
    border-radius: 8px;
    text-align: center;
    background: var(--panel-2);
    color: var(--muted);
    border: 1px solid var(--border);
  }
  .item.info .sev {
    color: var(--accent);
  }
  .item.warn {
    background: var(--warn-bg);
  }
  .item.warn .sev {
    color: var(--warn);
    border-color: var(--warn);
  }
  .item.dim {
    color: var(--muted);
  }
  .text {
    font-family: var(--mono);
    font-size: 12.5px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .sub {
    margin-top: 2px;
    font-size: 12px;
    color: var(--muted);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .sub b {
    font-weight: 600;
    margin-right: 4px;
  }
  .ctx {
    font-family: var(--mono);
  }
  .at {
    font-size: 11px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    padding-top: 2px;
  }
</style>
