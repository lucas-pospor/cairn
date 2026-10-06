<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../app.svelte";
  import { closeOnBack } from "../back";
  import { backend } from "../backend";
  import type { HistoryEntry } from "../types";
  import { displayName } from "../paths";
  import { modal } from "../modal";
  import Icon from "./Icon.svelte";

  let { path }: { path: string } = $props();
  let entries = $state<HistoryEntry[]>([]);
  let selected = $state<number | null>(null);
  let preview = $state<string>("");
  let error = $state<string | null>(null);
  let loading = $state(true);

  $effect(() => {
    backend
      .syncHistory(path)
      .then((h) => {
        entries = h;
        if (h[0]) void pick(h[0].seq);
      })
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  });

  // A deletion has no text: there is nothing to restore from it.
  const selectedDeleted = $derived(entries.find((e) => e.seq === selected)?.deleted ?? false);

  async function pick(seq: number) {
    selected = seq;
    preview = "";
    if (selectedDeleted) {
      preview = "This version is a deletion. It has no text to restore.";
      return;
    }
    try {
      const r = await backend.syncRevision(seq);
      preview = r.text;
    } catch (e) {
      preview = `Could not load this version: ${e}`;
    }
  }

  async function restore() {
    if (selected == null || selectedDeleted) return;
    const ok = await app.confirm({
      title: "Restore version",
      message: `Replace the current text of ${displayName(path)} with this version? The current text stays in the history.`,
      okLabel: "Restore",
    });
    if (!ok) return;
    try {
      const tab = app.tabs.find((t) => t.path === path);
      if (tab) await app.flush(tab);
      await backend.syncRestore(path, selected);
      app.toast("Version restored.");
      app.historyFor = null;
    } catch (e) {
      app.toast(String(e), "error");
    }
  }

  function close() {
    app.historyFor = null;
  }

  const opener = app.takeOpener();
  let closeButton: HTMLButtonElement;
  onMount(() => {
    closeButton.focus();
    return closeOnBack(close);
  });
</script>

<div
  class="backdrop"
  role="presentation"
  onmousedown={(e) => {
    if (e.target === e.currentTarget) close();
  }}
>
  <div class="history" role="dialog" aria-modal="true" aria-label="Version history" use:modal={{ close, opener }} data-testid="history">
    <header>
      <Icon name="history" />
      <h3>Version history: {displayName(path)}</h3>
      <button class="icon-btn" title="Close" bind:this={closeButton} onclick={close}><Icon name="x" /></button>
    </header>
    <div class="body">
      <ul>
        {#if loading}<li class="muted">Loading…</li>{/if}
        {#if error}<li class="err">{error}</li>{/if}
        {#each entries as e (e.seq)}
          <li>
            <button class:on={selected === e.seq} aria-current={selected === e.seq || undefined} onclick={() => pick(e.seq)} data-testid="history-entry">
              <span>{new Date(e.created * 1000).toLocaleString()}</span>
              <span class="muted small">{e.deleted ? "deleted" : `${e.device}`}</span>
            </button>
          </li>
        {/each}
      </ul>
      <pre class="preview">{preview}</pre>
    </div>
    <footer>
      <span class="muted small">Versions are kept on the sync server.</span>
      <button class="btn primary" onclick={restore} disabled={selected == null || selected === entries[0]?.seq || selectedDeleted} data-testid="history-restore">Restore this version</button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.3);
    z-index: 96;
    display: grid;
    place-items: center;
    padding: 16px;
  }
  .history {
    width: min(900px, 100%);
    height: min(600px, 100%);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: var(--shadow);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  footer {
    border-top: 1px solid var(--border);
    border-bottom: none;
    justify-content: space-between;
  }
  h3 {
    flex: 1;
    margin: 0;
    font-size: 15px;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 240px 1fr;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 6px;
    overflow: auto;
    border-right: 1px solid var(--border);
  }
  li button {
    display: flex;
    flex-direction: column;
    width: 100%;
    text-align: left;
    padding: 6px 10px;
    border-radius: 6px;
  }
  li button:hover {
    background: var(--bg-hover);
  }
  li button.on {
    background: var(--accent-soft);
    /* In the high-contrast themes, a ring as well as the tint. */
    box-shadow: inset 0 0 0 var(--ring) var(--accent);
  }
  /* Focused as well: the ring that shows the selection, and the focus ring. */
  li button.on:focus-visible {
    box-shadow:
      inset 0 0 0 var(--ring) var(--accent),
      0 0 0 var(--ring) var(--accent);
  }
  .small {
    font-size: 12px;
  }
  .preview {
    margin: 0;
    padding: 14px 18px;
    overflow: auto;
    font-family: var(--font-mono);
    font-size: 13px;
    white-space: pre-wrap;
    user-select: text;
  }
  .err {
    color: var(--danger);
  }
</style>
