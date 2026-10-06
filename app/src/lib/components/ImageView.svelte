<script lang="ts">
  // An image tab. The file loads through vault://, as images in notes do,
  // and only into an <img>: an SVG shown there runs none of its scripts.
  import { tick } from "svelte";
  import { app, type Tab } from "../app.svelte";
  import { extension, fileName } from "../paths";

  let { tab }: { tab: Tab } = $props();

  /** Space around the image, in px. */
  const PAD = 16;
  let body: HTMLDivElement;
  let stage: HTMLDivElement;
  let img: HTMLImageElement;
  let status = $state<"loading" | "shown" | "failed">("loading");
  let natural = $state({ w: 0, h: 0 });
  /** The path `natural` was measured for: a reused tab may still show the image before. */
  let shownPath = $state<string | null>(null);
  /** Room for the image inside the stage. */
  let room = $state({ w: 0, h: 0 });

  let name = $derived(fileName(tab.path));
  // A new URL after each change on disk makes the web view load the file again.
  let src = $derived(app.imageSrc(tab.path));
  let entry = $derived(app.entries.find((e) => e.kind === "file" && e.path === tab.path));
  // The size of this file, once it has loaded (while it loads again after a
  // change, the one before). An SVG without a width and height has none: it
  // is always fitted.
  let sized = $derived(shownPath === tab.path && status !== "failed" && natural.w > 0 && natural.h > 0);
  let fit = $derived(sized && room.w > 0 && room.h > 0 ? Math.min(1, room.w / natural.w, room.h / natural.h) : 1);
  /** Fitting shrinks it: actual size shows more. */
  let zoomable = $derived(sized && fit < 1);
  let actual = $derived(tab.actualSize && zoomable);
  let scale = $derived(actual ? 1 : fit);
  // Below 100% exactly when Actual size shows more, and never 0%.
  let percent = $derived(scale < 1 ? Math.min(99, Math.max(1, Math.round(scale * 100))) : 100);
  let info = $derived([sized ? `${natural.w} × ${natural.h} px` : "", sized ? `${percent}%` : "", entry ? size(entry.size) : ""].filter(Boolean).join(" · "));

  $effect(() => {
    void src;
    status = "loading";
  });

  // The room is measured on the stage's box, which scrollbars do not change,
  // and set in the next frame: a resize it causes must not loop.
  $effect(() => {
    const measure = () => (room = { w: body.clientWidth - 2 * PAD, h: body.clientHeight - 2 * PAD });
    // Now, before the image loads: it must not show at full size first.
    measure();
    let frame = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    });
    ro.observe(body);
    return () => {
      ro.disconnect();
      cancelAnimationFrame(frame);
    };
  });

  // Take the keyboard focus when shown and when asked (app.imageFocus), as
  // the editor does for a note, unless something covers the tab.
  $effect(() => {
    void app.imageFocus;
    const id = requestAnimationFrame(() => {
      if (!app.renaming && !app.switcherOpen && !app.paletteOpen && !app.dialog && !app.settingsOpen && !app.historyFor) stage.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(id);
  });

  function loaded() {
    if (extension(tab.path) !== "svg") return show(img.naturalWidth, img.naturalHeight);
    // WebKit gives an SVG the size it is laid out at as its natural size:
    // ask a copy that is not on the page.
    const at = src;
    const copy = new Image();
    copy.onload = () => at === src && show(copy.naturalWidth, copy.naturalHeight);
    copy.onerror = () => at === src && show(0, 0);
    copy.src = at;
  }

  function show(w: number, h: number) {
    natural = { w, h };
    shownPath = tab.path;
    status = "shown";
  }

  /**
   * Switch between fitted and actual size. The point at `at` (viewport
   * coordinates; the middle of the stage by default) stays where it is.
   */
  async function toggle(at?: { x: number; y: number }) {
    if (!zoomable && !tab.actualSize) return;
    const s = stage.getBoundingClientRect();
    const r = img.getBoundingClientRect();
    const x = at?.x ?? s.left + s.width / 2;
    const y = at?.y ?? s.top + s.height / 2;
    const fx = Math.min(1, Math.max(0, (x - r.left) / (r.width || 1)));
    const fy = Math.min(1, Math.max(0, (y - r.top) / (r.height || 1)));
    tab.actualSize = !tab.actualSize;
    await tick();
    const r2 = img.getBoundingClientRect();
    stage.scrollLeft += r2.left + fx * r2.width - x;
    stage.scrollTop += r2.top + fy * r2.height - y;
  }

  /** Keys scroll an image larger than the tab (also where the web view would not on its own). */
  function onKey(e: KeyboardEvent) {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const page = stage.clientHeight * 0.9;
    const by: Record<string, [number, number]> = {
      ArrowUp: [0, -40],
      ArrowDown: [0, 40],
      ArrowLeft: [-40, 0],
      ArrowRight: [40, 0],
      PageUp: [0, -page],
      PageDown: [0, page],
      Home: [0, -stage.scrollHeight],
      End: [0, stage.scrollHeight],
    };
    const d = by[e.key];
    if (!d) return;
    e.preventDefault();
    stage.scrollBy(d[0], d[1]);
  }

  function size(n: number): string {
    if (n < 1024) return `${n} bytes`;
    if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<div class="image-view" data-testid="image-view">
  <div class="bar">
    <span class="info" title={info} data-testid="image-info">{info}</span>
    <button class="btn" aria-pressed={actual} disabled={!zoomable && !actual} onclick={() => toggle()} data-testid="image-actual-size">Actual size</button>
    {#if !app.isMobile}
      <button class="btn" onclick={() => app.openAttachment(tab.path)} data-testid="image-open-default">Open in default app</button>
    {/if}
  </div>
  <div class="body" bind:this={body}>
  <!-- Focusable so that the keys scroll an image larger than the tab. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div class="stage" class:zoomable class:actual bind:this={stage} tabindex="0" role="region" aria-label={name} onkeydown={onKey} data-testid="image-stage">
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
    <img
      bind:this={img}
      {src}
      alt={name}
      draggable="false"
      class:free={!sized}
      class:hidden={status === "failed" || shownPath !== tab.path}
      style={sized ? `width: ${Math.floor(natural.w * scale)}px; height: ${Math.floor(natural.h * scale)}px` : undefined}
      onload={loaded}
      onerror={() => (status = "failed")}
      onclick={(e) => toggle({ x: e.clientX, y: e.clientY })}
      data-testid="image"
    />
  </div>
  {#if status === "failed"}
    <div class="state" role="alert" data-testid="image-failed">
      {#if entry}
        <p>Cairn cannot show {name}.</p>
        <p class="muted">The file may be damaged, or in a format this system cannot display.</p>
      {:else}
        <p>{name} is not in the vault any more.</p>
      {/if}
    </div>
  {:else if status === "loading"}
    <div class="state loading muted">Loading…</div>
  {/if}
  </div>
</div>

<style>
  .image-view {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
  }
  .bar .btn {
    padding: 3px 10px;
  }
  .bar .btn[aria-pressed="true"] {
    background: var(--accent-soft);
    border-color: var(--accent);
  }
  .bar .btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
  /* One line: a wrap would change the room, and so the scale it shows. */
  .info {
    flex: 1;
    min-width: 0;
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .body {
    flex: 1;
    min-height: 0;
    position: relative;
  }
  .stage {
    height: 100%;
    overflow: auto;
    display: flex;
    padding: 16px;
  }
  .stage:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  img {
    /* Centred; an image larger than the stage still scrolls from its top left. */
    margin: auto;
    flex: none;
    display: block;
    /* Shows where an image is transparent, in both themes. */
    background-color: var(--bg);
    background-image: conic-gradient(var(--bg-hover) 25%, transparent 0 50%, var(--bg-hover) 0 75%, transparent 0);
    background-size: 16px 16px;
  }
  img.free {
    width: 100%;
    height: 100%;
    object-fit: contain;
    background: none;
  }
  img.hidden {
    visibility: hidden;
  }
  .zoomable img {
    cursor: zoom-in;
  }
  .zoomable.actual img {
    cursor: zoom-out;
  }
  .state {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    text-align: center;
    padding: 16px;
    pointer-events: none;
  }
  .state p {
    margin: 4px;
  }
  .loading {
    /* Only for a slow load: no flash on a fast one. */
    animation: appear 0s 300ms both;
  }
  @keyframes appear {
    from {
      visibility: hidden;
    }
  }
</style>
