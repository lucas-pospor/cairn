<script lang="ts" module>
  export interface MenuItem {
    label: string;
    action: () => void;
    danger?: boolean;
    separatorBefore?: boolean;
  }
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../app.svelte";
  import { closeOnBack } from "../back";
  import { popup, restoreFocus } from "../modal";

  let { x, y, items, onclose }: { x: number; y: number; items: MenuItem[]; onclose: () => void } = $props();
  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ left: 0, top: 0 });
  // Taken before any effect runs: the element focused before the menu opened.
  // It gets focus back when the menu closes.
  const opener = document.activeElement;
  // The item that is the menu's Tab stop: the focused one.
  let current = $state(0);

  $effect(() => {
    // keep the menu inside the window
    const w = el?.offsetWidth ?? 200;
    const h = el?.offsetHeight ?? 200;
    pos = { left: Math.min(x, window.innerWidth - w - 8), top: Math.min(y, window.innerHeight - h - 8) };
  });

  // A modal overlay opened with a hotkey closes the menu: only one at a time.
  $effect(() => {
    if (app.switcherOpen || app.paletteOpen || app.settingsOpen || app.historyFor || app.dialog) onclose();
  });

  /** Up and Down move through the items, Home and End go to the first and last; Tab closes the menu. */
  function onKey(e: KeyboardEvent) {
    if (!el || e.ctrlKey || e.altKey || e.metaKey) return;
    // WebKitGTK gives Shift+Tab the key "Unidentified" (code "Tab").
    if (e.key === "Tab" || e.code === "Tab") {
      e.preventDefault();
      onclose();
      return;
    }
    const buttons = [...el.querySelectorAll<HTMLElement>("[role=menuitem]")];
    const i = buttons.indexOf(document.activeElement as HTMLElement);
    let next: number;
    switch (e.key) {
      case "ArrowDown":
        next = (i + 1) % buttons.length;
        break;
      case "ArrowUp":
        next = i <= 0 ? buttons.length - 1 : i - 1;
        break;
      case "Home":
      case "PageUp":
        next = 0;
        break;
      case "End":
      case "PageDown":
        next = buttons.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    buttons[next]?.focus();
  }

  onMount(() => {
    // Like any menu, it takes focus on its first item when it opens.
    el?.querySelector<HTMLElement>("[role=menuitem]")?.focus();
    const down = (e: MouseEvent) => {
      if (el && !el.contains(e.target as Node)) onclose();
    };
    // Escape closes the menu wherever focus is, and only the menu: it is used
    // up before the drawer the menu may be in sees it.
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      onclose();
    };
    setTimeout(() => window.addEventListener("mousedown", down), 0);
    window.addEventListener("keydown", key, true);
    window.addEventListener("blur", onclose);
    const offBack = closeOnBack(() => onclose());
    return () => {
      window.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("blur", onclose);
      offBack();
    };
  });
</script>

<!-- Focus is on the items (one Tab stop), not on the menu. -->
<!-- svelte-ignore a11y_interactive_supports_focus -->
<div
  class="menu"
  role="menu"
  bind:this={el}
  use:popup={opener}
  style="left: {pos.left}px; top: {pos.top}px"
  onkeydown={onKey}
  oncontextmenu={(e) => e.preventDefault()}
>
  {#each items as item, i}
    {#if item.separatorBefore}<div class="sep" role="separator"></div>{/if}
    <button
      role="menuitem"
      tabindex={i === current ? 0 : -1}
      class:danger={item.danger}
      onfocus={() => (current = i)}
      onpointermove={(e) => {
        // The item under the mouse is the one Enter runs.
        if (e.pointerType === "mouse") e.currentTarget.focus();
      }}
      onclick={() => {
        onclose();
        // Back to where focus was, so a dialog the item opens returns it there.
        restoreFocus(opener);
        item.action();
      }}>{item.label}</button
    >
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 150;
    min-width: 180px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 9px;
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  button {
    text-align: left;
    padding: 6px 10px;
    border-radius: 5px;
  }
  button:hover,
  button:focus-visible {
    background: var(--accent-soft);
    /* In the high-contrast themes, a ring as well as the tint. */
    box-shadow: inset 0 0 0 var(--ring) var(--accent);
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .danger {
    color: var(--danger);
  }
  .sep {
    height: 1px;
    background: var(--border);
    margin: 4px 2px;
  }
</style>
