<script lang="ts">
  import { tick } from "svelte";
  import { app, type Tab, type ViewMode } from "../app.svelte";
  import Icon from "./Icon.svelte";

  const modes: { mode: ViewMode; icon: string; label: string }[] = [
    { mode: "live", icon: "pencil", label: "Live Preview" },
    { mode: "source", icon: "code", label: "Source" },
    { mode: "split", icon: "columns", label: "Source and preview side by side" },
    { mode: "preview", icon: "eye", label: "Reading view" },
  ];

  let tabsEl: HTMLDivElement;
  let newButton: HTMLButtonElement;
  let newKey = $derived(app.hotkeyHint("note:new"));

  // The tabs are one Tab stop: the focused tab while focus is among them,
  // else the open tab (else the first). Tabs hold no other controls: the
  // close mark is for the pointer, Delete closes the focused tab.
  let focused = $state<number | null>(null);
  let stop = $derived(app.tabs.find((t) => t.id === focused)?.id ?? app.active?.id ?? app.tabs[0]?.id);

  async function close(tab: Tab) {
    const i = app.tabs.indexOf(tab);
    await app.closeTab(tab);
    await tick();
    // The focused tab went away: stay in the tab bar.
    const now = document.activeElement;
    if (now && now !== document.body && now.isConnected) return;
    const tabs = tabsEl.querySelectorAll<HTMLElement>("[role=tab]");
    (tabs[i] ?? tabs[i - 1] ?? newButton).focus();
  }

  function onTabKey(e: KeyboardEvent, tab: Tab) {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const tabs = tabsEl.querySelectorAll<HTMLElement>("[role=tab]");
    const i = app.tabs.indexOf(tab);
    const n = tabs.length;
    let to = -1;
    if (e.key === "ArrowRight") to = (i + 1) % n;
    else if (e.key === "ArrowLeft") to = (i - 1 + n) % n;
    else if (e.key === "Home") to = 0;
    else if (e.key === "End") to = n - 1;
    else if (e.key === "Enter" || e.key === " ") {
      // On the open tab, go back to its editor or image (as in the file tree).
      if (app.activeId !== tab.id) app.activate(tab);
      else if (tab.kind === "note" && tab.mode !== "preview") app.view?.focus();
      else if (tab.kind === "image") app.imageFocus++;
    } else if (e.key === "Delete") void close(tab);
    else return;
    e.preventDefault();
    tabs[to]?.focus();
  }
</script>

<div class="tabbar">
  <div
    class="tabs"
    role="tablist"
    aria-label="Open files"
    bind:this={tabsEl}
    onfocusout={(e) => {
      if (!tabsEl.contains(e.relatedTarget as Node | null)) focused = null;
    }}
  >
    {#each app.tabs as tab (tab.id)}
      <div
        class="tab"
        class:active={app.activeId === tab.id}
        role="tab"
        tabindex={tab.id === stop ? 0 : -1}
        aria-selected={app.activeId === tab.id}
        title={tab.path}
        data-testid="tab"
        data-path={tab.path}
        onclick={() => app.activate(tab)}
        onauxclick={(e) => {
          if (e.button === 1) app.closeTab(tab);
        }}
        onfocus={() => (focused = tab.id)}
        onkeydown={(e) => onTabKey(e, tab)}
      >
        <span class="label">{tab.title}</span>
        {#if tab.dirty}<span class="dot" title="Unsaved changes"></span>{/if}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <span
          class="close"
          title="Close"
          aria-hidden="true"
          onclick={(e) => {
            e.stopPropagation();
            void close(tab);
          }}><Icon name="x" size={13} /></span
        >
      </div>
    {/each}
  </div>
  <button class="icon-btn new" title={newKey ? `New note (${newKey})` : "New note"} bind:this={newButton} onclick={() => app.newNote()}><Icon name="file-plus" size={15} /></button>
  {#if app.active?.kind === "note"}
    <div class="modes" role="group" aria-label="View mode">
      {#each modes as m}
        <button
          class="icon-btn"
          class:on={app.active.mode === m.mode}
          title={m.label}
          aria-pressed={app.active.mode === m.mode}
          data-testid="mode-{m.mode}"
          onclick={() => app.setMode(m.mode)}><Icon name={m.icon} /></button
        >
      {/each}
    </div>
  {/if}
</div>

<style>
  .tabbar {
    display: flex;
    align-items: center;
    height: 40px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-side);
    padding-right: 8px;
  }
  .tabs {
    flex: 0 1 auto;
    min-width: 0;
    display: flex;
    align-items: flex-end;
    height: 100%;
    overflow-x: auto;
    overflow-y: hidden;
    padding-left: 6px;
    gap: 2px;
    scrollbar-width: none;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    min-width: 90px;
    max-width: 220px;
    padding: 0 6px 0 12px;
    border-radius: 8px 8px 0 0;
    color: var(--text-muted);
    cursor: default;
    flex: 0 1 auto;
  }
  .tab:hover {
    background: var(--bg-hover);
  }
  .tab:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .tab.active {
    background: var(--bg);
    color: var(--text);
    box-shadow: 0 -1px 0 var(--border), 1px 0 0 var(--border), -1px 0 0 var(--border);
  }
  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    flex: none;
  }
  /* 24px square: the smallest pointer target WCAG 2.5.8 allows. */
  .close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: 24px;
    height: 24px;
    border-radius: 4px;
    color: var(--text-faint);
    cursor: pointer;
    opacity: 0;
  }
  .tab:hover .close,
  .tab.active .close,
  .tab:focus-visible .close {
    opacity: 1;
  }
  .close:hover {
    background: var(--bg-active);
    color: var(--text);
  }
  .new {
    flex: none;
    margin-left: 2px;
  }
  .modes {
    margin-left: auto;
    display: flex;
    gap: 2px;
  }
</style>
