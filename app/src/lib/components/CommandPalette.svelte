<script lang="ts">
  import { onMount } from "svelte";
  import { closeOnBack } from "../back";
  import fuzzysort from "fuzzysort";
  import { app } from "../app.svelte";
  import { commands, displayCombo } from "../commands";
  import { modal } from "../modal";

  let query = $state("");
  let index = $state(0);
  let input: HTMLInputElement;
  let list: HTMLDivElement | undefined = $state();

  const all = commands.all().sort((a, b) => a.name.localeCompare(b.name));
  let results = $derived(query.trim() ? fuzzysort.go(query.trim(), all, { key: "name", limit: 50 }).map((r) => r.obj) : all);

  // Focus stays in the input; aria-activedescendant tells screen readers
  // which command is highlighted.
  const uid = $props.id();

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
    app.paletteOpen = false;
  }

  function run(i: number) {
    const c = results[i];
    close();
    if (c) setTimeout(() => c.run(), 0);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      index = Math.min(index + 1, results.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      index = Math.max(index - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(index);
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
  <div class="palette" role="dialog" aria-modal="true" aria-label="Command palette" use:modal={{ close, opener }}>
    <input
      bind:this={input}
      bind:value={query}
      class="q"
      placeholder="Type a command…"
      role="combobox"
      aria-label="Find a command"
      aria-autocomplete="list"
      aria-expanded={results.length > 0}
      aria-controls="{uid}-list"
      aria-activedescendant={results.length ? `${uid}-${index}` : undefined}
      onkeydown={onKey}
      data-testid="palette-input"
    />
    <div class="list" bind:this={list} id="{uid}-list" role="listbox" aria-label="Commands">
      {#each results as c, i (c.id)}
        <button
          class="item"
          class:sel={i === index}
          data-i={i}
          id="{uid}-{i}"
          role="option"
          aria-selected={i === index}
          onmousemove={() => (index = i)}
          onclick={() => run(i)}
          data-testid="palette-item"
        >
          <span>{c.name}</span>
          <span class="keys">
            {#each commands.keysFor(c.id).slice(0, 1) as k}<kbd>{displayCombo(k)}</kbd>{/each}
          </span>
        </button>
      {/each}
    </div>
    {#if !results.length}
      <p class="muted none">No matching commands.</p>
    {/if}
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
  .palette {
    width: min(560px, calc(100vw - 32px));
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    max-height: 64vh;
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
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    border-radius: 7px;
    text-align: left;
  }
  .item.sel {
    background: var(--accent-soft);
  }
  kbd {
    font-family: var(--font-mono);
    font-size: 11.5px;
    color: var(--text-muted);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 5px;
  }
  .none {
    padding: 8px 12px;
  }
</style>
