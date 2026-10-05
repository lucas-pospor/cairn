<script lang="ts">
  import { tick } from "svelte";
  import { app, tagQuery } from "../app.svelte";
  import { backend } from "../backend";
  import type { TagCount } from "../types";

  let tags = $state<TagCount[]>([]);
  let filter = $state("");
  let sortBy = $state<"count" | "name">("count");

  $effect(() => {
    void app.changeSeq;
    backend.tags().then((t) => (tags = t)).catch(() => (tags = []));
  });

  // Tags come folded like search::fold in the core: NFC, lowercase, "İ" as "i".
  let needle = $derived(filter.normalize("NFC").toLowerCase().replace(/i̇/g, "i").replace(/^#/, ""));
  let shown = $derived(
    tags
      .filter((t) => !needle || t.tag.includes(needle))
      .sort((a, b) => (sortBy === "name" ? a.tag.localeCompare(b.tag) : b.count - a.count || a.tag.localeCompare(b.tag))),
  );

  async function open(tag: string) {
    app.searchQuery = tagQuery(tag);
    app.leftPanel = "search";
    // This panel (and the focused tag) is gone now: continue in the search box.
    await tick();
    document.querySelector<HTMLInputElement>("[data-testid=search-input]")?.focus();
  }
</script>

<div class="head">
  <input class="text-input" placeholder="Filter tags" aria-label="Filter tags" bind:value={filter} />
  <button class="sort" onclick={() => (sortBy = sortBy === "count" ? "name" : "count")} title="Change sort order">
    {sortBy === "count" ? "By count" : "By name"}
  </button>
</div>
<div class="list" data-testid="tags-list">
  {#each shown as t (t.tag)}
    <button class="tag-row" onclick={() => open(t.tag)} data-testid="tag-row">
      <span class="name">#{t.tag}</span>
      <span class="count">{t.count}</span>
    </button>
  {/each}
  {#if !shown.length}
    <p class="muted empty">{tags.length ? "No tags match." : "No tags yet. Add #tags in notes or a tags: list in frontmatter."}</p>
  {/if}
</div>

<style>
  .head {
    display: flex;
    gap: 6px;
    padding: 10px 10px 6px;
  }
  .sort {
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
    padding: 0 6px;
    border-radius: 6px;
  }
  .sort:hover {
    background: var(--bg-hover);
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 0 6px 24px;
  }
  .tag-row {
    display: flex;
    width: 100%;
    align-items: center;
    padding: 5px 8px;
    border-radius: 6px;
    text-align: left;
  }
  .tag-row:hover {
    background: var(--bg-hover);
  }
  .name {
    flex: 1;
    color: var(--accent);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .count {
    font-size: 12px;
    color: var(--text-muted);
    background: var(--bg-hover);
    border-radius: 9px;
    padding: 0 7px;
  }
  .empty {
    padding: 8px;
    line-height: 1.5;
  }
</style>
