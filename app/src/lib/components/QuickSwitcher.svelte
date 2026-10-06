<script lang="ts">
  import { onMount } from "svelte";
  import { closeOnBack } from "../back";
  import fuzzysort from "fuzzysort";
  import { app } from "../app.svelte";
  import { displayName, isImage, isMarkdown, parent } from "../paths";
  import { modal } from "../modal";
  import { switcherStep } from "../overlayKeys";

  let query = $state("");
  let index = $state(0);
  let input: HTMLInputElement;
  let list: HTMLDivElement | undefined = $state();

  // Notes and images (they open in a tab). Prepared once per open; fuzzysort caches the preparation.
  const files = app.entries
    .filter((e) => e.kind === "file" && (isMarkdown(e.path) || isImage(e.path)))
    .map((e) => ({ path: e.path, name: displayName(e.path), mtime: e.mtime }));
  // Before anything is typed: the recent notes only, so that a batch of new
  // images (a camera folder, a sync) does not push them out.
  const recentFirst = files.filter((f) => isMarkdown(f.path)).sort((a, b) => b.mtime - a.mtime);

  let results = $derived(
    query.trim()
      ? fuzzysort.go(query.trim(), files, { keys: ["name", "path"], limit: 60 }).map((r) => r.obj)
      : recentFirst.slice(0, 60),
  );
  let exact = $derived(results.some((r) => r.name.toLowerCase() === query.trim().toLowerCase()));
  let showCreate = $derived(!!query.trim() && !exact);

  // Focus stays in the input; aria-activedescendant tells screen readers
  // which option is highlighted.
  const uid = $props.id();
  let activeId = $derived(results.length ? `${uid}-${index}` : showCreate ? `${uid}-create` : undefined);

  $effect(() => {
    void results;
    index = 0;
  });

  $effect(() => {
    list?.querySelector(`[data-i="${index}"]`)?.scrollIntoView({ block: "nearest" });
  });

  const opener = app.takeOpener();

  onMount(() => {
    input.focus();
    return closeOnBack(close);
  });

  function close() {
    app.switcherOpen = false;
  }

  function choose(i: number, newTab: boolean) {
    const r = results[i];
    close();
    if (!r) return;
    // The list is made when the switcher opens: the note may be gone since.
    if (app.entries.some((e) => e.path === r.path)) app.openNote(r.path, { newTab });
    else app.toast(`"${r.name}" was deleted or moved.`);
  }

  function create() {
    const name = query.trim();
    close();
    if (name) app.createNoteNamed(name);
  }

  function onKey(e: KeyboardEvent) {
    const mod = app.isMac ? e.metaKey : e.ctrlKey;
    // Ctrl+N / Ctrl+P too; Workspace leaves them to us (overlayKeys.ts).
    const step = switcherStep(e);
    if (e.key === "ArrowDown" || step === 1) {
      e.preventDefault();
      index = Math.min(index + 1, Math.max(results.length - 1, 0));
    } else if (e.key === "ArrowUp" || step === -1) {
      e.preventDefault();
      index = Math.max(index - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (e.shiftKey || results.length === 0) create();
      else choose(index, mod);
    }
  }
</script>

<div
  class="backdrop"
  role="presentation"
  onmousedown={(e) => {
    if (e.target === e.currentTarget) close();
  }}
>
  <div class="switcher" role="dialog" aria-modal="true" aria-label="Quick switcher" use:modal={{ close, opener }}>
    <input
      bind:this={input}
      bind:value={query}
      class="q"
      placeholder="Find or create a note…"
      role="combobox"
      aria-label="Find or create a note"
      aria-autocomplete="list"
      aria-expanded={results.length > 0 || showCreate}
      aria-controls="{uid}-list"
      aria-activedescendant={activeId}
      onkeydown={onKey}
      data-testid="switcher-input"
    />
    <div class="list" bind:this={list} id="{uid}-list" role="listbox" aria-label="Notes and images">
      {#each results as r, i (r.path)}
        <button
          class="item"
          class:sel={i === index}
          data-i={i}
          id="{uid}-{i}"
          role="option"
          aria-selected={i === index}
          onmousemove={() => (index = i)}
          onclick={(e) => choose(i, app.isMac ? e.metaKey : e.ctrlKey)}
          data-testid="switcher-item"
        >
          <span class="name">{r.name}</span>
          {#if parent(r.path)}<span class="dir">{parent(r.path)}</span>{/if}
        </button>
      {/each}
      {#if showCreate}
        <button
          class="item create"
          class:sel={results.length === 0}
          id="{uid}-create"
          role="option"
          aria-selected={results.length === 0}
          onclick={create}
          data-testid="switcher-create"
        >
          <span>Create note “{query.trim()}”</span>
          <span class="dir">Shift+Enter</span>
        </button>
      {/if}
    </div>
    <div class="hints">
      <span><kbd>↑↓</kbd> move</span><span><kbd>↵</kbd> open</span><span><kbd>{app.isMac ? "⌘" : "Ctrl"}+↵</kbd> new tab</span><span
        ><kbd>Shift+↵</kbd> create</span
      ><span><kbd>Esc</kbd> close</span>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.18);
    z-index: 90;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
  }
  .switcher {
    width: min(620px, calc(100vw - 32px));
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    max-height: 70vh;
  }
  .q {
    border: none;
    outline: none;
    background: transparent;
    padding: 16px 18px;
    font-size: 16px;
    border-bottom: 1px solid var(--border);
  }
  .list {
    overflow: auto;
    padding: 6px;
  }
  .item {
    display: flex;
    width: 100%;
    gap: 10px;
    align-items: baseline;
    padding: 7px 12px;
    border-radius: 7px;
    text-align: left;
  }
  .item.sel {
    background: var(--accent-soft);
  }
  .name {
    font-weight: 550;
  }
  .dir {
    color: var(--text-faint);
    font-size: 12.5px;
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .create {
    color: var(--accent);
  }
  .hints {
    display: flex;
    gap: 14px;
    flex-wrap: wrap;
    padding: 8px 14px;
    border-top: 1px solid var(--border);
    font-size: 11.5px;
    color: var(--text-muted);
    background: var(--bg-side);
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 11px;
  }
</style>
