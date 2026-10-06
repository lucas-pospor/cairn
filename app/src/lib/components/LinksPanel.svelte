<script lang="ts">
  import { app, tagQuery } from "../app.svelte";
  import { backend } from "../backend";
  import { displayName, parent } from "../paths";
  import type { Backlinks, NoteInfo, OutgoingLink } from "../types";
  import Icon from "./Icon.svelte";

  let tab = $state<"links" | "outline" | "properties">("links");
  let info = $state<NoteInfo | null>(null);
  let backlinks = $state<Backlinks[]>([]);
  let outgoing = $state<OutgoingLink[]>([]);
  let timer: ReturnType<typeof setTimeout> | undefined;

  // A note, or an image: the notes that link to it or embed it.
  let linked = $derived(app.active?.kind === "note" || app.active?.kind === "image" ? app.active : null);

  $effect(() => {
    const path = linked?.path ?? null;
    const note = linked?.kind === "note";
    void app.changeSeq;
    clearTimeout(timer);
    if (!path) {
      backlinks = [];
      outgoing = [];
      info = null;
      return;
    }
    timer = setTimeout(async () => {
      try {
        const [b, o, i]: [Backlinks[], OutgoingLink[], NoteInfo | null] = note
          ? await Promise.all([backend.backlinks(path), backend.outgoingLinks(path), backend.noteInfo(path)])
          : [await backend.backlinks(path), [], null];
        if (app.active?.path === path) {
          backlinks = b;
          outgoing = o;
          info = i;
        }
      } catch {
        backlinks = [];
        outgoing = [];
        info = null;
      }
    }, 80);
  });

  let total = $derived(backlinks.reduce((n, b) => n + b.items.length, 0));
  let uniqueOut = $derived(
    outgoing.filter((o, i) => outgoing.findIndex((x) => (x.resolved ?? x.target) === (o.resolved ?? o.target)) === i),
  );
</script>

<div class="side-tabs">
  <button class="icon-btn" class:on={tab === "links"} aria-pressed={tab === "links"} title="Links" onclick={() => (tab = "links")} data-testid="right-links"><Icon name="link" /></button>
  <button class="icon-btn" class:on={tab === "outline"} aria-pressed={tab === "outline"} title="Outline" onclick={() => (tab = "outline")} data-testid="right-outline"><Icon name="list" /></button>
  <button class="icon-btn" class:on={tab === "properties"} aria-pressed={tab === "properties"} title="Properties and tags" onclick={() => (tab = "properties")} data-testid="right-properties"
    ><Icon name="tag" /></button
  >
</div>
{#if tab === "outline"}
  <div class="panel" data-testid="outline">
    {#if !info || !info.headings.length}
      <p class="muted pad">{app.active?.kind === "note" ? "This note has no headings." : "Open a note to see its outline."}</p>
    {:else}
      {#each info.headings as h}
        <button class="heading" style="padding-left: {8 + (h.level - 1) * 14}px" onclick={() => app.active && app.openNote(app.active.path, { line: h.line })}
          >{h.text || "(empty heading)"}</button
        >
      {/each}
    {/if}
  </div>
{:else if tab === "properties"}
  <div class="panel" data-testid="properties">
    <h2 class="head">Properties</h2>
    {#if app.active?.kind !== "note"}
      <p class="muted pad">Open a note to see its properties.</p>
    {:else if info?.frontmatter && Object.keys(info.frontmatter).length}
      {#each Object.entries(info.frontmatter) as [k, v]}
        <div class="prop">
          <span class="prop-key">{k}</span>
          <span class="prop-val">{Array.isArray(v) ? v.join(", ") : typeof v === "object" && v !== null ? JSON.stringify(v) : String(v)}</span>
        </div>
      {/each}
    {:else if info?.frontmatterInvalid}
      <p class="muted pad">The <code>---</code> block at the top of the note is not valid YAML, so its properties cannot be shown.</p>
    {:else}
      <p class="muted pad">No frontmatter. Add a <code>---</code> block at the top of the note to set properties.</p>
    {/if}
    <h2 class="head out">Tags</h2>
    {#if app.active?.kind !== "note"}
      <p class="muted pad">Open a note to see its tags.</p>
    {:else if info?.tags.length}
      <div class="tags">
        {#each info.tags as t}
          <button class="tag" onclick={() => ((app.searchQuery = tagQuery(t)), (app.leftPanel = "search"), (app.leftOpen = true))}>#{t}</button>
        {/each}
      </div>
    {:else}
      <p class="muted pad">No tags.</p>
    {/if}
  </div>
{:else}
<div class="panel" data-testid="backlinks">
  <h2 class="head">
    <Icon name="link" size={14} />
    <span>Backlinks</span>
    {#if linked}<span class="count">{total}</span>{/if}
  </h2>
  {#if !linked}
    <p class="muted pad">Open a note to see what links to it.</p>
  {:else if backlinks.length === 0}
    <p class="muted pad">{linked.kind === "image" ? "No notes link to" : "No other notes link to"} {linked.title}.</p>
  {:else}
    {#each backlinks as group (group.source)}
      <div class="group">
        <button class="src" onclick={() => app.openNote(group.source)} data-testid="backlink-source">
          {displayName(group.source)}
          {#if parent(group.source)}<span class="dir muted">{parent(group.source)}</span>{/if}
        </button>
        {#each group.items as item}
          <button class="ctx" onclick={() => app.openNote(group.source, { line: item.line })}>{item.context}</button>
        {/each}
      </div>
    {/each}
  {/if}

  {#if app.active?.kind === "note" && uniqueOut.length}
    <h2 class="head out">
      <Icon name="arrow" size={14} />
      <span>Outgoing links</span>
      <span class="count">{uniqueOut.length}</span>
    </h2>
    {#each uniqueOut as o}
      <button
        class="outlink"
        class:unresolved={!o.resolved}
        title={o.resolved ? o.resolved : "Not created yet: click to create"}
        onclick={() => app.openLink(o.target, null, false, o.kind)}>{o.resolved ? displayName(o.resolved) : o.target}</button
      >
    {/each}
  {/if}
</div>
{/if}

<style>
  .side-tabs {
    display: flex;
    gap: 2px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    height: 40px;
    align-items: center;
  }
  .heading {
    display: block;
    width: 100%;
    text-align: left;
    padding: 4px 8px;
    border-radius: 6px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .heading:hover {
    background: var(--bg-hover);
  }
  .prop {
    display: grid;
    grid-template-columns: minmax(70px, 35%) 1fr;
    gap: 8px;
    padding: 4px 8px;
    font-size: 13px;
  }
  .prop-key {
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .prop-val {
    overflow-wrap: anywhere;
    user-select: text;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 4px 8px;
  }
  .tag {
    color: var(--accent);
    background: var(--accent-soft);
    border-radius: 10px;
    padding: 1px 8px;
    font-size: 12.5px;
  }
  .panel {
    flex: 1;
    overflow: auto;
    padding: 4px 8px 24px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    margin: 0;
    padding: 0 6px;
    font-size: 11.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  .head.out {
    margin-top: 12px;
    border-top: 1px solid var(--border);
  }
  .count {
    margin-left: auto;
    font-weight: 500;
    background: var(--bg-hover);
    border-radius: 9px;
    padding: 0 7px;
    letter-spacing: 0;
  }
  .pad {
    padding: 4px 8px;
    line-height: 1.5;
  }
  .group {
    margin-bottom: 8px;
  }
  .src,
  .outlink {
    display: flex;
    gap: 8px;
    align-items: baseline;
    width: 100%;
    text-align: left;
    padding: 5px 8px;
    border-radius: 6px;
    font-weight: 600;
  }
  .outlink {
    font-weight: 450;
    color: var(--link);
  }
  /* Dashed, as in the editor, so it does not differ by colour alone. */
  .outlink.unresolved {
    color: var(--unresolved);
    text-decoration: underline dashed;
    text-underline-offset: 3px;
  }
  .dir {
    font-size: 12px;
    font-weight: 400;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ctx {
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
  .src:hover,
  .ctx:hover,
  .outlink:hover {
    background: var(--bg-hover);
  }
</style>
