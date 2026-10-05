<script lang="ts">
  import { app } from "../app.svelte";
  import Icon from "./Icon.svelte";
  import { commands } from "../commands";

  // Inert while a drawer covers it (small screens).
  let { inert = false }: { inert?: boolean } = $props();
  let words = $state(0);

  const skipped = $derived(app.sync?.skipped ?? []);
  const syncedLabel = $derived.by(() => {
    const parts = ["Synced"];
    const n = skipped.length;
    if (n) parts.push(`${n} ${n > 1 ? "files" : "file"} not synced`);
    const c = app.sync?.conflicts.length ?? 0;
    if (c) parts.push(`${c} conflict${c > 1 ? "s" : ""}`);
    return parts.join(" · ");
  });
  let timer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    void app.docSeq;
    const tab = app.active;
    void tab?.loading;
    clearTimeout(timer);
    timer = setTimeout(() => {
      if (!tab) {
        words = 0;
        return;
      }
      const text = app.docOf(tab);
      words = (text.match(/[\p{L}\p{N}][\p{L}\p{N}'’-]*/gu) ?? []).length;
    }, 250);
  });

  let syncLabel = $derived.by(() => {
    const st = app.sync;
    if (!st) return "";
    if (st.state === "syncing") return "Syncing…";
    if (st.state === "error") return "Sync error";
    return syncedLabel;
  });

  // Screen readers hear the sync changes that matter (an error, the fix,
  // new conflicts), not every "Syncing…": a sync runs seconds after each edit.
  let syncNews = $state("");
  let settled: string | null = null;
  $effect(() => {
    if (!app.sync?.configured || app.sync.state === "syncing") return;
    if (settled !== null && syncLabel !== settled) syncNews = syncLabel;
    settled = syncLabel;
  });
</script>

<footer class="status" {inert}>
  <button class="vault" title="Switch vault" aria-label="Switch vault (current: {app.vault?.name})" onclick={() => app.closeVault()}>
    <Icon name="vault" size={13} />
    {app.vault?.name}
  </button>
  <span class="spacer"></span>
  {#if app.sync?.configured}
    <button
      class="sync {app.sync.state}"
      title={app.sync.lastError ?? `Last sync: ${app.sync.lastSync ? new Date(app.sync.lastSync).toLocaleTimeString() : "never"}.${skipped.length ? ` Not synced: ${skipped[0].path || "a file from the server"}${skipped.length > 1 ? ` and ${skipped.length - 1} more` : ""}.` : ""} Click to sync now.`}
      onclick={() => commands.run("sync:now")}
      data-testid="sync-indicator"
    >
      <Icon name={app.sync.state === "error" ? "cloud-off" : "cloud"} size={13} />
      {syncLabel}
    </button>
  {/if}
  <span class="sr-only" role="status" data-testid="sync-news">{syncNews}</span>
  {#if app.active?.kind === "note"}
    <span>{words.toLocaleString()} {words === 1 ? "word" : "words"}</span>
    <span class="save" data-testid="save-state">
      {#if app.active.conflict}
        <span class="warn">Not saved: conflict</span>
      {:else if app.active.dirty}
        Unsaved
      {:else}
        Saved
      {/if}
    </span>
  {/if}
  <button
    class="icon-btn small"
    title="Toggle left sidebar"
    aria-expanded={app.leftOpen}
    aria-controls="left-sidebar"
    onclick={() => (app.leftOpen = !app.leftOpen)}><Icon name="panel-left" size={13} /></button
  >
  <button
    class="icon-btn small"
    title="Toggle right sidebar"
    aria-expanded={app.rightOpen}
    aria-controls="right-sidebar"
    onclick={() => (app.rightOpen = !app.rightOpen)}><Icon name="panel-right" size={13} /></button
  >
</footer>

<style>
  .status {
    grid-column: 1 / -1;
    padding-bottom: env(safe-area-inset-bottom);
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 8px;
    border-top: 1px solid var(--border);
    background: var(--bg-side);
    color: var(--text-muted);
    font-size: 12px;
  }
  .vault {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 24px;
    padding: 2px 6px;
    border-radius: 5px;
  }
  .vault:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .sync {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-height: 24px;
    padding: 2px 6px;
    border-radius: 5px;
  }
  .sync:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .sync.error {
    color: var(--danger);
  }
  .sync.syncing {
    color: var(--accent);
  }
  .spacer {
    flex: 1;
  }
  .warn {
    color: var(--danger);
  }
  /* 24px: the smallest pointer target WCAG 2.5.8 allows. */
  .small {
    width: 24px;
    height: 24px;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
