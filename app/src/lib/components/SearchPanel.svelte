<script lang="ts">
  import { app } from "../app.svelte";
  import { backend } from "../backend";
  import { displayName, fileName, parent } from "../paths";
  import { matchImages } from "../imageSearch";
  import type { SearchHit } from "../types";

  const MAX_IMAGES = 100;
  let results = $state<SearchHit[]>([]);
  /** Images whose path matches (search covers note text only). */
  let images = $state<string[]>([]);
  let searching = $state(false);
  let elapsed = $state(0);
  let counts = $derived(
    [
      results.length === 200 ? "200+ results" : results.length === 1 ? "1 result" : `${results.length} results`,
      ...(images.length ? [images.length === MAX_IMAGES ? `${MAX_IMAGES}+ images` : images.length === 1 ? "1 image" : `${images.length} images`] : []),
      `${elapsed.toFixed(0)} ms`,
    ].join(" · "),
  );
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
      images = [];
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
          images = matchImages(app.filePaths, q, MAX_IMAGES);
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
      else if (e.key === "Enter" && images[0]) app.openNote(images[0]);
    }}
  />
  <!-- Always present, so screen readers announce the counts as they change. -->
  <div class="meta muted" role="status">
    {#if app.searchQuery.trim()}
      {#if searching && !results.length && !images.length}Searching…{:else}{counts}{/if}
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
  {#if images.length}
    <h2 class="group muted" id="search-images">Images</h2>
    <div role="group" aria-labelledby="search-images">
      {#each images as p (p)}
        <div class="hit">
          <button class="file" onclick={(e) => app.openNote(p, { newTab: app.isMac ? e.metaKey : e.ctrlKey })} data-testid="search-image">
            <span class="name">{fileName(p)}</span>
            {#if parent(p)}<span class="dir muted">{parent(p)}</span>{/if}
          </button>
        </div>
      {/each}
    </div>
  {/if}
  {#if app.searchQuery.trim() && !searching && !results.length && !images.length}
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
  .group {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin: 12px 8px 4px;
  }
</style>
