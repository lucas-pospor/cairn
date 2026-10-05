<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../app.svelte";
  import FileTree from "./FileTree.svelte";
  import SearchPanel from "./SearchPanel.svelte";
  import TabBar from "./TabBar.svelte";
  import EditorPane from "./EditorPane.svelte";
  import LinksPanel from "./LinksPanel.svelte";
  import QuickSwitcher from "./QuickSwitcher.svelte";
  import StatusBar from "./StatusBar.svelte";
  import Icon from "./Icon.svelte";
  import TagsPanel from "./TagsPanel.svelte";
  import CommandPalette from "./CommandPalette.svelte";
  import SettingsModal from "./SettingsModal.svelte";
  import HistoryModal from "./HistoryModal.svelte";
  import { commands, comboFromEvent } from "../commands";
  import { overlayKeepsKey } from "../overlayKeys";
  import { recordingHotkey } from "../hotkeyRecorder";
  import { closeOnBack } from "../back";
  import { drawer } from "../modal";

  const load = (k: string, d: number) => {
    try {
      return Number(localStorage.getItem(k)) || d;
    } catch {
      return d;
    }
  };
  let leftW = $state(load("cairn.leftW", 260));
  let rightW = $state(load("cairn.rightW", 280));
  // Small screen: an open sidebar is a modal drawer over the rest, which is inert.
  let leftDrawer = $derived(app.narrow && app.leftOpen);
  let rightDrawer = $derived(app.narrow && app.rightOpen);
  let covered = $derived(leftDrawer || rightDrawer);

  function startResize(side: "left" | "right", e: PointerEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const start = side === "left" ? leftW : rightW;
    const move = (ev: PointerEvent) => {
      const d = ev.clientX - startX;
      const w = Math.min(560, Math.max(180, side === "left" ? start + d : start - d));
      if (side === "left") leftW = w;
      else rightW = w;
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      try {
        localStorage.setItem("cairn.leftW", String(leftW));
        localStorage.setItem("cairn.rightW", String(rightW));
      } catch {}
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  // Keys of the editor's search panel (CodeMirror's searchKeymap). While it
  // has focus they stay with it: Ctrl+G there is "find next", not the graph.
  const SEARCH_PANEL_KEYS = ["Mod+F", "Mod+G", "Mod+Shift+G", "F3", "Shift+F3"];

  function onKey(e: KeyboardEvent) {
    // No commands while a modal is open: they would act behind it.
    if (app.dialog || app.settingsOpen || app.historyFor || recordingHotkey()) return;
    const combo = comboFromEvent(e);
    if (!combo) return;
    if (SEARCH_PANEL_KEYS.includes(combo) && e.target instanceof Element && e.target.closest(".cm-search")) return;
    const cmd = commands.lookup(combo);
    if (!cmd) return;
    // The switcher and palette are modal too, but may be swapped for one
    // another. Ctrl+N / Ctrl+P stay with the switcher (next / previous result).
    const overlay = app.switcherOpen ? "switcher" : app.paletteOpen ? "palette" : null;
    if (overlayKeepsKey(overlay, cmd.id, e)) return;
    // Editor commands only apply while typing in the editor.
    // (A key event means the window has focus, so only check the element.)
    if (cmd.id.startsWith("editor:") && !app.view?.contentDOM.contains(document.activeElement)) return;
    e.preventDefault();
    e.stopPropagation();
    void cmd.run();
  }

  // On a small screen the sidebars are drawers over the editor; Back closes them.
  $effect(() => {
    if (app.narrow && (app.leftOpen || app.rightOpen)) return closeOnBack(() => (app.leftOpen = app.rightOpen = false));
  });

  onMount(() => {
    window.addEventListener("keydown", onKey, true);
    const t = setInterval(() => app.saveSession(), 15000);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      clearInterval(t);
    };
  });
</script>

<div
  class="workspace"
  class:narrow={app.narrow}
  class:mobile={app.isMobile}
  style="--left-w: {app.leftOpen && !app.narrow ? leftW : 0}px; --right-w: {app.rightOpen && !app.narrow ? rightW : 0}px"
>
  {#if covered}
    <div
      class="drawer-backdrop"
      role="presentation"
      onclick={() => (app.leftOpen = app.rightOpen = false)}
    ></div>
  {/if}
  <aside
    class="left"
    class:hidden={!app.leftOpen}
    id="left-sidebar"
    data-testid="left-sidebar"
    role={leftDrawer ? "dialog" : undefined}
    aria-modal={leftDrawer || undefined}
    aria-label="Left sidebar"
    {@attach leftDrawer && drawer(() => (app.leftOpen = false))}
  >
    <div class="side-tabs">
      <button class="icon-btn" class:on={app.leftPanel === "files"} aria-pressed={app.leftPanel === "files"} title="Files" onclick={() => (app.leftPanel = "files")} data-testid="tab-files"
        ><Icon name="files" /></button
      >
      <button class="icon-btn" class:on={app.leftPanel === "search"} aria-pressed={app.leftPanel === "search"} title="Search" onclick={() => (app.leftPanel = "search")} data-testid="tab-search"
        ><Icon name="search" /></button
      >
      <button class="icon-btn" class:on={app.leftPanel === "tags"} aria-pressed={app.leftPanel === "tags"} title="Tags" onclick={() => (app.leftPanel = "tags")} data-testid="tab-tags"
        ><Icon name="tag" /></button
      >
      <span class="grow"></span>
      <button class="icon-btn" title="Graph view" onclick={() => app.openGraph()} data-testid="open-graph"><Icon name="graph" /></button>
      <button class="icon-btn" title="Command palette" onclick={() => app.openOverlay("palette")}><Icon name="command" /></button>
      <button class="icon-btn" title="Settings" onclick={() => app.openOverlay("settings")} data-testid="open-settings"><Icon name="settings" /></button>
    </div>
    <div class="side-body">
      {#if app.leftPanel === "files"}
        <FileTree />
      {:else if app.leftPanel === "search"}
        <SearchPanel />
      {:else}
        <TagsPanel />
      {/if}
    </div>
  </aside>
  {#if app.leftOpen && !app.narrow}
    <div class="resizer left-resizer" role="separator" aria-orientation="vertical" onpointerdown={(e) => startResize("left", e)}></div>
  {/if}

  <main class="center" inert={covered}>
    {#if app.narrow}
      <div class="mobile-bar">
        <button
          class="icon-btn big"
          title="Files"
          aria-expanded={app.leftOpen}
          aria-controls="left-sidebar"
          onclick={() => (app.leftOpen = !app.leftOpen)}
          data-testid="mobile-files"><Icon name="panel-left" size={20} /></button
        >
        <span class="mobile-title">{app.active?.title ?? app.vault?.name}</span>
        <button class="icon-btn big" title="Find note" onclick={() => app.openOverlay("switcher")}><Icon name="search" size={20} /></button>
        <button class="icon-btn big" title="New note" onclick={() => app.newNote()}><Icon name="file-plus" size={20} /></button>
        <button class="icon-btn big" title="Commands" onclick={() => app.openOverlay("palette")}><Icon name="command" size={20} /></button>
        <button
          class="icon-btn big"
          title="Links and outline"
          aria-expanded={app.rightOpen}
          aria-controls="right-sidebar"
          onclick={() => (app.rightOpen = !app.rightOpen)}><Icon name="panel-right" size={20} /></button
        >
      </div>
    {/if}
    <TabBar />
    <EditorPane />
  </main>

  {#if app.rightOpen && !app.narrow}
    <div class="resizer right-resizer" role="separator" aria-orientation="vertical" onpointerdown={(e) => startResize("right", e)}></div>
  {/if}
  <aside
    class="right"
    class:hidden={!app.rightOpen}
    id="right-sidebar"
    role={rightDrawer ? "dialog" : undefined}
    aria-modal={rightDrawer || undefined}
    aria-label="Right sidebar"
    {@attach rightDrawer && drawer(() => (app.rightOpen = false))}
  >
    <LinksPanel />
  </aside>

  <StatusBar inert={covered} />
</div>

{#if app.switcherOpen}
  <QuickSwitcher />
{/if}
{#if app.paletteOpen}
  <CommandPalette />
{/if}
{#if app.settingsOpen}
  <SettingsModal />
{/if}
{#if app.historyFor}
  <HistoryModal path={app.historyFor} />
{/if}

<style>
  .workspace {
    height: 100%;
    display: grid;
    grid-template-columns: var(--left-w) auto 1fr auto var(--right-w);
    grid-template-rows: 1fr 26px;
  }
  aside {
    background: var(--bg-side);
    min-width: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  aside.hidden {
    display: none;
  }
  .side-tabs {
    display: flex;
    gap: 2px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    height: 40px;
    align-items: center;
  }
  .grow {
    flex: 1;
  }
  .side-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .resizer {
    width: 1px;
    background: var(--border);
    cursor: col-resize;
    position: relative;
  }
  .resizer::after {
    content: "";
    position: absolute;
    inset: 0 -3px;
  }
  .resizer:hover {
    background: var(--accent);
  }
  .workspace.narrow {
    grid-template-columns: 0 0 1fr 0 0;
  }
  .workspace.narrow aside {
    position: fixed;
    top: 0;
    bottom: 0;
    z-index: 60;
    width: min(86vw, 340px);
    box-shadow: var(--shadow);
    padding-top: env(safe-area-inset-top);
    padding-bottom: env(safe-area-inset-bottom);
  }
  .workspace.narrow aside.left {
    left: 0;
  }
  .workspace.narrow aside.right {
    right: 0;
  }
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.3);
    z-index: 55;
  }
  .mobile-bar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: calc(4px + env(safe-area-inset-top)) 6px 4px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-side);
  }
  .mobile-title {
    flex: 1;
    min-width: 0;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 6px;
  }
  .icon-btn.big {
    width: 40px;
    height: 40px;
    flex: none;
  }
  /* Explicit columns: hidden or fixed-position siblings must not shift the editor. */
  aside.left {
    grid-column: 1;
    grid-row: 1;
  }
  .left-resizer {
    grid-column: 2;
    grid-row: 1;
  }
  .right-resizer {
    grid-column: 4;
    grid-row: 1;
  }
  aside.right {
    grid-column: 5;
    grid-row: 1;
  }
  .center {
    grid-column: 3;
    grid-row: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  /* A short phone screen (landscape with the keyboard up) keeps only the
     editor and its formatting toolbar, so the line being typed stays in view.
     The top bar holds the way to the files, so it goes only while typing. */
  @media (max-height: 300px) {
    .workspace.mobile {
      grid-template-rows: 1fr 0;
    }
    .workspace.mobile :global(.tabbar),
    .workspace.mobile > :global(.status) {
      display: none;
    }
    .workspace.mobile:has(:global(.cm-focused)) .mobile-bar {
      display: none;
    }
    .workspace.mobile :global(.cm-content) {
      padding-top: 6px;
    }
  }
</style>
