<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { EditorView } from "@codemirror/view";
  import { EditorState } from "@codemirror/state";
  import { app } from "../app.svelte";
  import { livePreviewCompartment } from "../editor/livePreview";
  import Preview from "./Preview.svelte";
  import GraphView from "./GraphView.svelte";
  import MobileToolbar from "./MobileToolbar.svelte";

  let host: HTMLDivElement;
  let view: EditorView | null = null;

  onMount(() => {
    view = new EditorView({ parent: host, state: EditorState.create({ doc: "" }) });
    app.view = view;
    // Handle for end-to-end tests.
    (view.dom as HTMLElement & { __cairnView?: EditorView }).__cairnView = view;
    return () => {
      app.stashActive();
      app.view = null;
      view?.destroy();
    };
  });

  // Show the active tab's document in the single editor view.
  $effect(() => {
    const tab = app.active;
    const loading = tab?.loading;
    void tab?.path;
    const mode = tab?.mode;
    if (!view || !tab || tab.kind !== "note" || loading || !tab.editorState) return;
    untrack(() => {
      if (app.viewTab !== tab) {
        app.showInView(tab);
        const top = tab.scrollTop;
        requestAnimationFrame(() => view && (view.scrollDOM.scrollTop = top));
      }
      // Live Preview on or off, per tab.
      const want = mode === "live" ? app.livePreview : [];
      if (livePreviewCompartment.get(view!.state) !== want) {
        view!.dispatch({ effects: livePreviewCompartment.reconfigure(want) });
      }
      app.docSeq++;
      if (tab.revealLine != null) requestAnimationFrame(() => app.reveal(tab));
      else if (mode !== "preview")
        requestAnimationFrame(() => {
          // Checked when the frame comes: the rename box or an overlay may have opened since.
          if (!app.renaming && !app.switcherOpen && !app.paletteOpen && !app.dialog && !app.settingsOpen && !app.historyFor) view?.focus();
        });
    });
  });

  // The editor must not take keys while it is hidden (reading view, graph)
  // or a modal covers it. WebKit types into the DOM selection even when focus
  // is on <body> or a button, so drop the focus and that selection.
  $effect(() => {
    const tab = app.active;
    const shown = tab?.kind === "note" && tab.mode !== "preview";
    const covered = app.settingsOpen || !!app.historyFor || !!app.dialog;
    if (!view || (shown && !covered)) return;
    const content = view.contentDOM;
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && content.contains(focused)) focused.blur();
    const sel = document.getSelection();
    if (sel?.anchorNode && content.contains(sel.anchorNode)) sel.removeAllRanges();
  });
</script>

<div class="pane">
  {#if app.active?.conflict}
    {@const tab = app.active}
    <div class="banner" role="alert" data-testid="conflict-banner">
      {#if tab.conflict === "deleted"}
        <span>This note was deleted or moved outside Cairn while you had unsaved edits.</span>
        <button class="btn" onclick={() => app.keepMine(tab)}>Save my version</button>
        <button class="btn" onclick={() => app.closeTab(tab, { skipSave: true })}>Discard and close</button>
      {:else}
        <span>This note changed on disk while you were editing it.</span>
        <button class="btn" onclick={() => app.loadTheirs(tab)} data-testid="conflict-theirs">Load disk version</button>
        <button class="btn" onclick={() => app.keepMine(tab)} data-testid="conflict-mine">Keep mine (overwrite)</button>
      {/if}
    </div>
  {/if}

  <div class="body mode-{app.active?.kind === 'graph' ? 'graph' : (app.active?.mode ?? 'none')}">
    <div class="editor" bind:this={host} data-testid="editor"></div>
    {#if app.active?.kind === "graph"}
      <div class="preview-wrap"><GraphView /></div>
    {:else if app.active && !app.active.loading && (app.active.mode === "preview" || app.active.mode === "split")}
      <div class="preview-wrap">
        <Preview tab={app.active} />
      </div>
    {/if}
  </div>

  {#if app.isMobile && app.active?.kind === "note" && app.active.mode !== "preview"}
    <MobileToolbar />
  {/if}

  {#if !app.active}
    <div class="empty">
      <p>No note open.</p>
      {#if app.isMobile}
        <p class="muted">Use the search button to find a note, or the new note button to start one.</p>
      {:else}
        {@const find = app.hotkeyHint("app:quick-switcher")}
        {@const create = app.hotkeyHint("note:new")}
        {#if find && create}
          <p class="muted"><kbd>{find}</kbd> to find a note, <kbd>{create}</kbd> to create one.</p>
        {:else if find}
          <p class="muted"><kbd>{find}</kbd> to find a note.</p>
        {:else if create}
          <p class="muted"><kbd>{create}</kbd> to create a note.</p>
        {/if}
      {/if}
    </div>
  {:else if app.active.error}
    <div class="empty"><p class="error">{app.active.error}</p></div>
  {/if}
</div>

<style>
  .pane {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    position: relative;
    background: var(--bg);
  }
  .banner {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 8px 12px;
    background: color-mix(in srgb, var(--danger) 12%, var(--bg));
    border-bottom: 1px solid color-mix(in srgb, var(--danger) 35%, var(--border));
  }
  .banner span {
    flex: 1 1 260px;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .editor {
    flex: 1;
    min-width: 0;
    height: 100%;
  }
  .preview-wrap {
    flex: 1;
    min-width: 0;
    height: 100%;
  }
  .mode-preview .editor,
  .mode-graph .editor,
  .mode-none .editor {
    display: none;
  }
  .mode-split .preview-wrap {
    border-left: 1px solid var(--border);
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    text-align: center;
    color: var(--text-muted);
    background: var(--bg);
  }
  .empty p {
    margin: 4px;
  }
  .error {
    color: var(--danger);
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 12px;
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: 4px;
    padding: 0 5px;
    background: var(--bg-side);
  }
</style>
