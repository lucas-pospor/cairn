<script lang="ts">
  import { app } from "../app.svelte";
  import { flatten, type TreeNode } from "../tree";
  import { displayName, fileName, isInside, isMarkdown, parent } from "../paths";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import Icon from "./Icon.svelte";
  import { backend } from "../backend";
  import { restoreFocus } from "../modal";

  const ROW_H = typeof matchMedia === "function" && matchMedia("(pointer: coarse)").matches ? 40 : 28;
  let rows = $derived(flatten(app.tree, app.expanded));
  // Only render the rows in view (folders can hold thousands of notes).
  let scrollTop = $state(0);
  let viewH = $state(800);
  let first = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - 10));
  let last = $derived(Math.min(rows.length, Math.ceil((scrollTop + viewH) / ROW_H) + 10));
  let visible = $derived(rows.slice(first, last));
  let selected = $state<string | null>(null);
  // The keyboard cursor, shown with aria-activedescendant (the tree keeps the
  // focus, rows come and go as they scroll): the selected row, else the open
  // note, else the first row. A row in a collapsed folder stands for the folder.
  let cursor = $derived.by(() => {
    for (const p of [selected, app.active?.path]) {
      if (!p || !app.entries.some((e) => e.path === p)) continue; // renamed or deleted
      for (let q = p; q; q = parent(q)) {
        const i = rows.findIndex((r) => r.node.path === q);
        if (i >= 0) return i;
      }
    }
    return rows.length ? 0 : -1;
  });
  let treeEl: HTMLDivElement;
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let renameValue = $state("");
  // Where focus was before the rename box took it; Escape gives it back.
  let renameOpener: Element | null = null;

  // ----- drag and drop (pointer based, works for mouse and touch) -----
  let drag = $state<{ path: string; x: number; y: number; active: boolean; target: string | null } | null>(null);
  let suppressClick = false;

  let longPress: ReturnType<typeof setTimeout> | undefined;

  function onPointerDown(e: PointerEvent, node: TreeNode) {
    if (e.button !== 0 || app.renaming) return;
    if (e.pointerType === "touch") {
      // Touch: a long press opens the menu (which has "Move to…"); dragging
      // would fight with scrolling.
      const x = e.clientX;
      const y = e.clientY;
      clearTimeout(longPress);
      longPress = setTimeout(() => {
        suppressClick = true;
        setTimeout(() => (suppressClick = false), 400);
        openMenu(new MouseEvent("contextmenu", { clientX: x, clientY: y }), node);
      }, 500);
      const cancel = () => clearTimeout(longPress);
      window.addEventListener("pointerup", cancel, { once: true });
      window.addEventListener("pointercancel", cancel, { once: true });
      return;
    }
    drag = { path: node.path, x: e.clientX, y: e.clientY, active: false, target: null };
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp, { once: true });
  }

  function dropTargetAt(x: number, y: number): string | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop]");
    if (!el) return null;
    const p = el.dataset.drop!;
    return el.dataset.kind === "dir" ? p : parent(p);
  }

  function onPointerMove(e: PointerEvent) {
    if (!drag) return;
    if (!drag.active && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 5) return;
    drag.active = true;
    drag.x = e.clientX;
    drag.y = e.clientY;
    const t = dropTargetAt(e.clientX, e.clientY);
    drag.target = t !== null && t !== drag.path && !isInside(t, drag.path) && t !== parent(drag.path) ? t : null;
  }

  async function onPointerUp() {
    window.removeEventListener("pointermove", onPointerMove);
    const d = drag;
    drag = null;
    if (!d?.active) return;
    suppressClick = true;
    setTimeout(() => (suppressClick = false), 0);
    if (d.target !== null) await app.moveInto(d.path, d.target);
  }

  // ----- clicks -----
  function onRowClick(e: MouseEvent, node: TreeNode) {
    if (suppressClick) return;
    activate(node, app.isMac ? e.metaKey : e.ctrlKey);
  }

  /** Click, Enter or Space on a row: open or close a folder, open a note. */
  function activate(node: TreeNode, newTab: boolean) {
    selected = node.path;
    if (node.kind === "dir") {
      toggle(node.path);
    } else if (isMarkdown(node.path)) {
      app.openNote(node.path, { newTab });
    } else {
      app.openAttachment(node.path);
    }
  }

  function toggle(dir: string) {
    if (app.expanded.has(dir)) app.expanded.delete(dir);
    else app.expanded.add(dir);
    app.saveSession();
  }

  function startRename(path: string) {
    app.renaming = path;
  }

  $effect(() => {
    const p = app.renaming;
    if (p) {
      const entry = app.entries.find((e) => e.path === p);
      renameValue = entry && entry.kind === "file" && isMarkdown(p) ? displayName(p) : fileName(p);
      const focused = document.activeElement;
      if (!(focused instanceof HTMLElement && focused.dataset.testid === "rename-input")) renameOpener = focused;
      queueMicrotask(() => {
        const input = document.querySelector<HTMLInputElement>("[data-testid=rename-input]");
        input?.focus();
        input?.select();
      });
    }
  });

  function onRenameKey(e: KeyboardEvent, path: string) {
    if (e.key === "Enter") {
      e.preventDefault();
      app.rename(path, renameValue);
      // On a small screen the drawer stays open over the editor: back to the tree.
      if (app.narrow) treeEl.focus();
    } else if (e.key === "Escape") {
      e.preventDefault();
      app.renaming = null;
      restoreFocus(app.narrow ? treeEl : renameOpener);
    }
  }

  function openMenu(e: MouseEvent, node: TreeNode | null) {
    e.preventDefault();
    clearTimeout(longPress);
    const items: MenuItem[] = [];
    const dir = node ? (node.kind === "dir" ? node.path : parent(node.path)) : "";
    if (node) selected = node.path;
    if (node?.kind === "file" && isMarkdown(node.path)) {
      items.push({ label: "Open in new tab", action: () => app.openNote(node.path, { newTab: true }) });
    } else if (node?.kind === "file" && !app.isMobile) {
      items.push({ label: "Open in default app", action: () => app.openAttachment(node.path) });
    }
    items.push({ label: "New note", action: () => app.newNote(dir), separatorBefore: items.length > 0 });
    items.push({ label: "New folder", action: () => app.newFolder(dir) });
    if (node?.kind === "file" && app.sync?.configured) {
      items.push({ label: "Version history", action: () => app.openHistory(node.path) });
    }
    if (node && !app.isMobile) {
      items.push({ label: "Reveal in file manager", action: () => backend.revealInFileManager(node.path) });
    }
    if (node) {
      items.push({ label: "Move to…", action: () => app.moveToFolderPrompt(node.path) });
      items.push({ label: "Rename…", action: () => startRename(node.path), separatorBefore: true });
      items.push({ label: "Delete", action: () => app.remove(node.path), danger: true });
    }
    menu = { x: e.clientX, y: e.clientY, items };
  }

  // ----- keyboard -----

  /** Scroll row `i` into view (rows outside the view are not rendered). */
  function reveal(i: number) {
    const top = 2 + i * ROW_H; // below the tree's top padding
    if (top < treeEl.scrollTop) treeEl.scrollTop = top;
    else if (top + ROW_H > treeEl.scrollTop + treeEl.clientHeight) treeEl.scrollTop = top + ROW_H - treeEl.clientHeight;
  }

  function moveTo(i: number) {
    const row = rows[i];
    if (!row) return;
    selected = row.node.path;
    menu = null;
    reveal(i);
  }

  /** The row menu for row `i`, below the row (Shift+F10, the Menu key). */
  function openRowMenu(i: number) {
    reveal(i);
    const box = treeEl.getBoundingClientRect();
    const bottom = box.top + 2 + (i + 1) * ROW_H - treeEl.scrollTop;
    openMenu(new MouseEvent("contextmenu", { clientX: box.left + 24, clientY: bottom }), rows[i].node);
  }

  // Whether the next "contextmenu" on the tree comes from a pointer (a right
  // click on the empty space: menu for the vault root) or from the keyboard
  // (menu for the row under the cursor).
  let pointerMenu = false;

  function onTreeContextMenu(e: MouseEvent) {
    if (!pointerMenu && rows[cursor]) {
      e.preventDefault();
      openRowMenu(cursor);
    } else openMenu(e, null);
    pointerMenu = false;
  }

  function onTreeKey(e: KeyboardEvent) {
    pointerMenu = false;
    // Keys typed in the rename box bubble up to here.
    if (e.target !== e.currentTarget || app.renaming) return;
    const row = rows[cursor];
    if (!row || e.altKey) return;
    const node = row.node;
    const mod = app.isMac ? e.metaKey : e.ctrlKey;
    // Leave other shortcuts to the hotkeys; Mod+Enter opens in a new tab.
    if ((e.ctrlKey || e.metaKey) && e.key !== "Enter") return;
    const open = node.kind === "dir" && app.expanded.has(node.path);
    switch (e.key) {
      case "ArrowDown":
        moveTo(cursor + 1);
        break;
      case "ArrowUp":
        moveTo(cursor - 1);
        break;
      case "Home":
        moveTo(0);
        break;
      case "End":
        moveTo(rows.length - 1);
        break;
      case "ArrowRight":
        // Open a folder, or go to its first child once it is open.
        if (node.kind === "dir" && !open) toggle(node.path);
        else if (open && rows[cursor + 1] && parent(rows[cursor + 1].node.path) === node.path) moveTo(cursor + 1);
        break;
      case "ArrowLeft":
        // Close a folder, or go to the parent folder.
        if (open) toggle(node.path);
        else if (parent(node.path)) moveTo(rows.findIndex((r) => r.node.path === parent(node.path)));
        break;
      case "Enter":
      case " ":
        selected = node.path;
        // The open note: go back to editing it (on a small screen openNote also closes the drawer).
        if (!mod && !app.narrow && app.active?.kind === "note" && app.active.path === node.path && app.active.mode !== "preview") app.view?.focus();
        else activate(node, mod);
        break;
      case "F2":
        startRename(node.path);
        break;
      case "Delete":
        app.remove(node.path);
        break;
      case "ContextMenu":
        openRowMenu(cursor);
        break;
      case "F10":
        if (!e.shiftKey) return;
        openRowMenu(cursor);
        break;
      default:
        return;
    }
    e.preventDefault();
  }
</script>

<div class="header">
  <h2 class="title">Files</h2>
  <button class="icon-btn" title="New note" onclick={() => app.newNote("")} data-testid="new-note"><Icon name="file-plus" /></button>
  <button class="icon-btn" title="New folder" onclick={() => app.newFolder("")} data-testid="new-folder"><Icon name="folder-plus" /></button>
  <button
    class="icon-btn"
    title="Collapse all"
    onclick={() => {
      app.expanded.clear();
      app.saveSession();
    }}><Icon name="collapse" /></button
  >
</div>

<!-- The tree is the one focus stop; rows are not focusable (see `cursor`). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  style="--row-h: {ROW_H}px"
  class="tree"
  role="tree"
  aria-label="Files"
  aria-activedescendant={cursor >= 0 ? `tree-row-${cursor}` : undefined}
  tabindex="0"
  data-drop=""
  data-kind="dir"
  class:drop-root={drag?.active && drag.target === ""}
  onpointerdown={() => (pointerMenu = true)}
  oncontextmenu={onTreeContextMenu}
  onkeydown={onTreeKey}
  onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
  bind:clientHeight={viewH}
  bind:this={treeEl}
  data-testid="file-tree"
>
  <div style="height: {first * ROW_H}px"></div>
  {#each visible as { node, depth, pos, size }, i (node.path)}
    {@const isActive = app.active?.path === node.path}
    <!-- svelte-ignore a11y_interactive_supports_focus, a11y_click_events_have_key_events -->
    <div
      class="row"
      class:active={isActive}
      class:selected={selected === node.path && !isActive}
      class:cursor={first + i === cursor}
      class:drop={drag?.active && drag.target === node.path}
      class:dragging={drag?.active && drag.path === node.path}
      class:non-note={node.kind === "file" && !isMarkdown(node.path)}
      id="tree-row-{first + i}"
      role="treeitem"
      aria-level={depth + 1}
      aria-setsize={size}
      aria-posinset={pos}
      aria-selected={first + i === cursor}
      aria-current={isActive ? "page" : undefined}
      aria-expanded={node.kind === "dir" ? app.expanded.has(node.path) : undefined}
      style="padding-left: {10 + depth * 14}px"
      data-drop={node.path}
      data-kind={node.kind}
      data-testid="tree-row"
      data-path={node.path}
      onpointerdown={(e) => onPointerDown(e, node)}
      onclick={(e) => onRowClick(e, node)}
      onauxclick={(e) => {
        if (e.button === 1 && node.kind === "file") app.openNote(node.path, { newTab: true });
      }}
      oncontextmenu={(e) => {
        e.stopPropagation();
        openMenu(e, node);
      }}
      ondblclick={() => startRename(node.path)}
    >
      {#if node.kind === "dir"}
        <span class="chev" class:open={app.expanded.has(node.path)}><Icon name="chevron" size={12} /></span>
      {:else}
        <span class="chev"></span>
      {/if}
      {#if app.renaming === node.path}
        <input
          class="rename"
          aria-label="New name"
          data-testid="rename-input"
          bind:value={renameValue}
          onkeydown={(e) => onRenameKey(e, node.path)}
          onblur={() => app.rename(node.path, renameValue)}
          onclick={(e) => e.stopPropagation()}
          onpointerdown={(e) => e.stopPropagation()}
        />
      {:else}
        <span class="name">{node.kind === "file" ? displayName(node.path) : node.name}</span>
        {#if node.kind === "file" && !isMarkdown(node.path)}
          <span class="ext">{node.name.split(".").pop()}</span>
        {/if}
      {/if}
    </div>
  {/each}
  <div style="height: {(rows.length - last) * ROW_H}px"></div>
  {#if rows.length === 0}
    {@const create = app.hotkeyHint("note:new")}
    <p class="empty muted">This vault is empty. Create a note with the button above{create ? ` or ${create}` : ""}.</p>
  {/if}
</div>

{#if drag?.active}
  <div class="ghost" style="left: {drag.x + 12}px; top: {drag.y + 6}px">{displayName(drag.path)}</div>
{/if}

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

<style>
  .header {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 8px 4px 12px;
  }
  .title {
    flex: 1;
    margin: 0;
    font-size: 11.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  .tree {
    flex: 1;
    overflow: auto;
    padding: 2px 6px 24px;
    outline: none;
  }
  .tree:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .tree:focus-visible .row.cursor {
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .tree.drop-root {
    background: var(--accent-soft);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    height: var(--row-h, 28px);
    padding-right: 8px;
    border-radius: 6px;
    cursor: default;
    white-space: nowrap;
    color: var(--text);
    touch-action: pan-y;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.active {
    background: var(--bg-active);
    font-weight: 550;
  }
  .row.selected {
    box-shadow: inset 0 0 0 1px var(--border-strong);
  }
  .row.drop {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .row.dragging {
    opacity: 0.45;
  }
  .row.non-note .name {
    color: var(--text-muted);
  }
  .chev {
    width: 14px;
    flex: none;
    display: inline-flex;
    color: var(--text-faint);
    transition: transform 0.12s;
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ext {
    font-size: 10.5px;
    text-transform: uppercase;
    /* Not --text-faint: the badge stays readable on hovered and open rows. */
    color: var(--text-muted);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 4px;
    margin-left: auto;
  }
  .rename {
    flex: 1;
    min-width: 0;
    padding: 2px 6px;
    border: 1px solid var(--accent);
    border-radius: 5px;
    background: var(--bg-input);
    outline: none;
  }
  .empty {
    padding: 12px;
    line-height: 1.5;
  }
  .ghost {
    position: fixed;
    pointer-events: none;
    z-index: 300;
    background: var(--bg-input);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    padding: 4px 10px;
    border-radius: 6px;
  }
</style>
