<script lang="ts">
  import { app, type Tab } from "../app.svelte";
  import { tick } from "svelte";
  import { renderMarkdown } from "../markdown";
  import { fillEmbeds } from "../embeds";
  import { embeddedImageAt } from "../opening";

  let { tab }: { tab: Tab } = $props();
  let html = $state("");
  let article: HTMLElement | undefined = $state();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let gen = 0;

  $effect(() => {
    void app.docSeq;
    void app.changeSeq; // link targets may have appeared or vanished
    void tab.loading;
    void tab.path;
    const first = html === "";
    clearTimeout(timer);
    timer = setTimeout(
      async () => {
        const my = ++gen;
        const out = await renderMarkdown(app.docOf(tab), { links: app.linkIndex, sourcePath: tab.path });
        if (my !== gen) return;
        html = out;
        await tick();
        if (article && my === gen) await fillEmbeds(article, tab.path, app.linkIndex);
      },
      first ? 0 : 150,
    );
    return () => clearTimeout(timer);
  });

  function onClick(e: MouseEvent) {
    // An image of the vault (not inside a link) opens in an image tab.
    const image = embeddedImageAt(e.target);
    if (image) {
      app.openEmbeddedImage(image, app.isMac ? e.metaKey : e.ctrlKey);
      return;
    }
    const a = (e.target as HTMLElement).closest("a");
    if (!a) return;
    e.preventDefault();
    if (a.classList.contains("internal-link")) {
      const href = a.dataset.href ?? "";
      const i = href.indexOf("#");
      const target = i >= 0 ? href.slice(0, i) : href;
      const sub = i >= 0 ? href.slice(i + 1) : null;
      app.openLink(target, sub, app.isMac ? e.metaKey : e.ctrlKey);
    } else if (a.classList.contains("tag")) {
      app.searchQuery = `#${a.dataset.tag}`;
      app.leftPanel = "search";
      app.leftOpen = true;
    } else {
      const href = a.getAttribute("href") ?? "";
      if (/^[a-z][a-z0-9+.-]*:/i.test(href)) app.openUrl(href);
      else if (href && !href.startsWith("#")) {
        const [p, sub] = href.split("#");
        app.openLink(decodeURIComponent(p), sub ? decodeURIComponent(sub) : null, false, "markdown");
      }
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="preview"
  onclick={onClick}
  onauxclick={(e) => {
    const image = e.button === 1 ? embeddedImageAt(e.target) : null;
    if (image) {
      e.preventDefault();
      app.openEmbeddedImage(image, true);
    }
  }}
  data-testid="preview"
>
  <article class="md-render" bind:this={article}>{@html html}</article>
</div>

<style>
  .preview {
    height: 100%;
    overflow: auto;
    user-select: text;
    -webkit-user-select: text;
  }
  .preview .md-render {
    max-width: var(--line-width);
    margin: 0 auto;
    padding: 28px 32px 30vh;
  }
  .preview :global(img[data-path]) {
    cursor: zoom-in;
  }
  .preview :global(a img[data-path]) {
    cursor: pointer;
  }
</style>
