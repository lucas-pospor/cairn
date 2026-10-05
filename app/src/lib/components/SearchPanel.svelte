<script lang="ts">
  import { app } from "../app.svelte";
  import { backend } from "../backend";
  import { displayName, parent } from "../paths";
  import type { SearchHit } from "../types";

  let results = $state<SearchHit[]>([]);
  let searching = $state(false);
  let elapsed = $state(0);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let gen = 0;
  /** The query the shown results belong to. */
  let searched = "";

  $effect(() => {
    const q = app.searchQuery;
    void app.changeSeq;
    clearTimeout(timer);
    // Results of an earlier run must not land after this one.
    const my = ++gen;
    if (!q.trim()) {
      results = [];
      searched = "";
      searching = false;
      return;
    }
    // A new query says "Searching…" from the first key, not "No matches."
    // while the debounce waits.
    if (q !== searched) searching = true;
    timer = setTimeout(async () => {
      const t0 = performance.now();
      try {
        const r = await backend.search(q, 200);
        if (my === gen) {
          results = r;
          searched = q;
          elapsed = performance.now() - t0;
        }
      } finally {
        if (my === gen) searching = false;
      }
    }, 120);
  });
</script>

<div class="search">
  <input
    class="text-input"
    placeholder="Search notes"
    aria-label="Search notes"
    bind:value={app.searchQuery}
    data-testid="search-input"
    onkeydown={(e) => {
      if (e.key === "Enter" && results[0]) app.openNote(results[0].path, { line: results[0].snippets[0]?.line });
    }}
  />
  <!-- Always present, so screen readers announce the counts as they change. -->
  <div class="meta muted" role="status">
    {#if app.searchQuery.trim()}
      {#if searching && !results.length}Searching…{:else}{results.length === 200 ? "200+ results" : results.length === 1 ? "1 result" : `${results.length} results`} · {elapsed.toFixed(0)} ms{/if}
    {/if}
  </div>
</div>
<div class="results" data-testid="search-results">
  {#each results as hit (hit.path)}
    <div class="hit">
      <button class="file" onclick={(e) => app.openNote(hit.path, { newTab: app.isMac ? e.metaKey : e.ctrlKey })} data-testid="search-hit">
        <span class="name">{displayName(hit.path)}</span>
        {#if parent(hit.path)}<span class="dir muted">{parent(hit.path)}</span>{/if}
      </button>
      {#each hit.snippets as sn}
        <button class="snippet" onclick={() => app.openNote(hit.path, { line: sn.line })}>
          {#each sn.segments as seg}{#if seg.hit}<mark class="hit">{seg.text}</mark>{:else}{seg.text}{/if}{/each}
        </button>
      {/each}
    </div>
  {/each}
  {#if app.searchQuery.trim() && !searching && !results.length}
    <p class="muted none">No matches.</p>
  {/if}
</div>

<style>
  .search {
    padding: 10px 10px 6px;
  }
  .meta {
    font-size: 12px;
    margin-top: 6px;
  }
  .meta:empty {
    margin-top: 0;
  }
  .results {
    flex: 1;
    overflow: auto;
    padding: 0 6px 24px;
  }
  .hit {
    margin-bottom: 6px;
  }
  .file {
    display: flex;
    gap: 8px;
    align-items: baseline;
    width: 100%;
    text-align: left;
    padding: 5px 8px;
    border-radius: 6px;
  }
  .file:hover,
  .snippet:hover {
    background: var(--bg-hover);
  }
  .name {
    font-weight: 600;
  }
  .dir {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .snippet {
    display: block;
    width: 100%;
    text-align: left;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--text-muted);
    padding: 4px 8px 4px 16px;
    border-radius: 6px;
    overflow-wrap: anywhere;
  }
  .none {
    padding: 8px;
  }
</style>
