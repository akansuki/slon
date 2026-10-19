<script lang="ts">
  import { onMount } from 'svelte';
  import { Compartment, EditorState, Prec } from '@codemirror/state';
  import { EditorView, keymap } from '@codemirror/view';
  import { basicSetup } from 'codemirror';
  import { PostgreSQL, sql } from '@codemirror/lang-sql';
  import { oneDark } from '@codemirror/theme-one-dark';
  import { lintGutter, setDiagnostics } from '@codemirror/lint';
  import { buildNamespace, columnCompletionSource } from './completion';
  import type { CompletionSchema } from './types';

  let {
    value,
    onchange,
    onrun,
    schema = null,
  }: {
    value: string;
    onchange: (v: string) => void;
    onrun: () => void;
    /** Names to complete; `null` until they are loaded. */
    schema?: CompletionSchema | null;
  } = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;

  const theme = new Compartment();
  const language = new Compartment();

  // Read by the column completion source at the moment completion runs.
  let completionInfo: CompletionSchema | null = null;

  const columnSource = columnCompletionSource(() => completionInfo);

  function languageFor(info: CompletionSchema | null) {
    if (!info) return sql({ dialect: PostgreSQL });
    const { schema: ns, defaultSchema } = buildNamespace(info);
    return sql({ dialect: PostgreSQL, schema: ns, defaultSchema });
  }

  // Tables and columns arrive after the editor exists (and again after a refresh).
  $effect(() => {
    completionInfo = schema;
    view?.dispatch({ effects: language.reconfigure(languageFor(schema)) });
  });

  /** The text to execute: the selection when there is one, otherwise everything. */
  export function getRunText(): { text: string; offset: number } {
    const sel = view!.state.selection.main;
    if (!sel.empty) return { text: view!.state.sliceDoc(sel.from, sel.to), offset: sel.from };
    return { text: view!.state.doc.toString(), offset: 0 };
  }

  /** Underline the error. `position` is the server's 1-based character offset into the run text. */
  export function showError(position: number, offset: number, message: string) {
    if (!view) return;
    const len = view.state.doc.length;
    if (len === 0) return;
    let from = Math.min(offset + position - 1, len - 1);
    from = Math.max(from, 0);
    const word = /^(?:[\w$]+|"[^"]*"?|.)/.exec(view.state.sliceDoc(from, Math.min(from + 80, len)));
    const to = Math.min(from + Math.max(word?.[0].length ?? 1, 1), len);
    view.dispatch(
      setDiagnostics(view.state, [{ from, to, severity: 'error', message }]),
      { selection: { anchor: from }, scrollIntoView: true },
    );
  }

  export function getText(): string {
    return view?.state.doc.toString() ?? value;
  }

  /** Replace the whole text (one undoable step, cursor at the end). */
  export function setText(text: string) {
    if (!view || view.state.doc.toString() === text) return;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, selection: { anchor: text.length }, scrollIntoView: true });
  }

  export function clearError() {
    view?.dispatch(setDiagnostics(view.state, []));
  }

  export function focus() {
    view?.focus();
  }

  onMount(() => {
    const media = matchMedia('(prefers-color-scheme: dark)');
    const pick = () => (media.matches ? oneDark : []);

    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: value,
        extensions: [
          // Must beat the default keymap, which binds Mod-Enter to "insert blank line".
          Prec.highest(
            keymap.of([
              { key: 'Mod-Enter', run: () => (onrun(), true) },
              { key: 'F5', run: () => (onrun(), true) },
            ]),
          ),
          basicSetup,
          language.of(languageFor(schema)),
          // Bare column names of the tables the current statement uses.
          // One stable function: CodeMirror keys its running completion state by source identity.
          EditorState.languageData.of(() => [{ autocomplete: columnSource }]),
          lintGutter(),
          theme.of(pick()),
          EditorView.theme({
            '&': { height: '100%', fontSize: '13px', backgroundColor: 'var(--panel)' },
            '&.cm-focused': { outline: 'none' },
            '.cm-scroller': { fontFamily: 'var(--mono)', lineHeight: '1.5' },
            '.cm-gutters': { backgroundColor: 'var(--panel-2)', borderRight: '1px solid var(--border)' },
          }),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) onchange(u.state.doc.toString());
          }),
        ],
      }),
    });

    const onTheme = () => view?.dispatch({ effects: theme.reconfigure(pick()) });
    media.addEventListener('change', onTheme);
    return () => {
      media.removeEventListener('change', onTheme);
      view?.destroy();
    };
  });
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
    overflow: hidden;
  }
</style>
