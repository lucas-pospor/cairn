<script lang="ts">
  import { onMount } from "svelte";
  import Graph from "graphology";
  import Sigma from "sigma";
  import FA2Layout from "graphology-layout-forceatlas2/worker";
  import forceAtlas2 from "graphology-layout-forceatlas2";
  import { app } from "../app.svelte";
  import { backend } from "../backend";
  import { settings } from "../settings.svelte";
  import { displayName } from "../paths";
  import type { GraphData } from "../types";
  import Icon from "./Icon.svelte";

  let container: HTMLDivElement;
  let graph = new Graph({ multi: false, type: "directed" });
  let renderer: Sigma | null = null;
  let layout: FA2Layout | null = null;
  let stopTimer: ReturnType<typeof setTimeout> | undefined;
  let hovered: string | null = null;
  /**
   * What the hovered node changes, worked out once per hover: the reducers
   * below run for every node and edge on each refresh.
   */
  let near = new Set<string>();
  let nearEdges = new Set<string>();
  let theme = colors();
  /** The colours of the nodes, edges and labels drawn so far. */
  let drawn = theme;
  /** The node picked from the keyboard (find box or arrow keys); Enter opens it. */
  let picked = $state<string | null>(null);
  /** The find box text that picked it, so a second Enter opens it. */
  let pickedFor: string | null = null;
  let query = $state("");
  let stats = $state({ nodes: 0, edges: 0 });
  let running = $state(false);
  let loading = $state(true);
  let error = $state<string | null>(null);

  function css(name: string, fallback: string) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
  }

  function colors() {
    return {
      note: css("--text-muted", "#888"),
      active: css("--accent", "#a84529"),
      unresolved: css("--unresolved", "#6f675f"),
      edge: css("--border", "#ccc"),
      label: css("--text", "#222"),
      dim: css("--bg-active", "#ddd"),
    };
  }

  let recolorTimer: ReturnType<typeof setTimeout> | undefined;
  /**
   * After a change of theme, accent or snippet, draw with the new colours. A
   * moment later: settings re-add snippets one file read at a time, and the
   * colours in between are not worth redrawing a large graph for.
   */
  function scheduleRecolor() {
    clearTimeout(recolorTimer);
    recolorTimer = setTimeout(recolor, 100);
  }

  function recolor() {
    const c = colors();
    const changed = (...keys: (keyof typeof c)[]) => keys.some((k) => c[k] !== drawn[k]);
    const nodes = changed("note", "unresolved");
    const edges = changed("edge");
    const label = changed("label");
    const reducers = changed("active", "dim");
    drawn = theme = c;
    // Only the colour changes, which tells sigma it need not index the graph again.
    if (nodes) graph.updateEachNodeAttributes((_, a) => ({ ...a, color: a.kind === "note" ? c.note : c.unresolved }), { attributes: ["color"] });
    if (edges) graph.updateEachEdgeAttributes((_, a) => ({ ...a, color: c.edge }), { attributes: ["color"] });
    if (label) renderer?.setSetting("labelColor", { color: c.label });
    else if (reducers && !nodes && !edges) renderer?.refresh({ skipIndexation: true });
  }

  function nodeSize(degree: number) {
    return 1.5 + Math.min(9, Math.sqrt(degree) * 0.9);
  }

  /** Merge fresh data into the graph, keeping positions of known nodes. */
  function apply(data: GraphData) {
    const c = colors();
    const keep = new Set<string>();
    const n = data.nodes.length;
    data.nodes.forEach((node, i) => {
      const key = node.kind === "note" ? node.id : `?${node.id}`;
      keep.add(key);
      const attrs = {
        label: node.kind === "note" ? displayName(node.id) : node.id,
        size: nodeSize(node.degree),
        color: node.kind === "note" ? c.note : c.unresolved,
        kind: node.kind,
        path: node.id,
      };
      if (graph.hasNode(key)) graph.mergeNodeAttributes(key, attrs);
      else {
        // Start on a spiral so the layout has something to work with.
        const a = i * 2.39996;
        const r = Math.sqrt(i / Math.max(n, 1)) * 100;
        graph.addNode(key, { ...attrs, x: Math.cos(a) * r, y: Math.sin(a) * r });
      }
    });
    graph.forEachNode((k) => {
      if (!keep.has(k)) graph.dropNode(k);
    });
    const keyOf = (i: number) => {
      const node = data.nodes[i];
      return node.kind === "note" ? node.id : `?${node.id}`;
    };
    const keepEdges = new Set<string>();
    for (const [a, b] of data.edges) {
      const s = keyOf(a);
      const t = keyOf(b);
      const id = `${s}->${t}`;
      keepEdges.add(id);
      if (!graph.hasEdge(id)) graph.addDirectedEdgeWithKey(id, s, t, { size: 0.6, color: c.edge });
    }
    graph.forEachEdge((id) => {
      if (!keepEdges.has(id)) graph.dropEdge(id);
    });
    stats = { nodes: graph.order, edges: graph.size };
  }

  function runLayout(seconds?: number) {
    layout?.kill();
    clearTimeout(stopTimer);
    if (graph.order === 0) return;
    const inferred = forceAtlas2.inferSettings(graph);
    layout = new FA2Layout(graph, {
      settings: { ...inferred, barnesHutOptimize: graph.order > 1000, slowDown: 2, gravity: 1 },
    });
    layout.start();
    running = true;
    const secs = seconds ?? Math.min(12, 2 + graph.order / 1500);
    stopTimer = setTimeout(() => {
      layout?.stop();
      running = false;
    }, secs * 1000);
  }

  async function load() {
    try {
      const data = await backend.graph(settings.value.graphShowUnresolved);
      const first = graph.order === 0;
      apply(data);
      // The hovered node's links may have changed, and the theme with them.
      hover(hovered && graph.hasNode(hovered) ? hovered : null);
      loading = false;
      if (first) runLayout();
      renderer?.refresh();
    } catch (e) {
      error = String((e as { detail?: string })?.detail ?? e);
      loading = false;
    }
  }

  function hover(node: string | null) {
    hovered = node;
    near = new Set(node ? [node, ...graph.neighbors(node)] : []);
    nearEdges = new Set(node ? graph.edges(node) : []);
    theme = colors();
  }

  function focusNode(key: string) {
    if (!renderer || !graph.hasNode(key)) return;
    const pos = renderer.getNodeDisplayData(key);
    if (pos) renderer.getCamera().animate({ x: pos.x, y: pos.y, ratio: 0.25 }, { duration: 400 });
    hover(key);
    renderer.refresh();
  }

  function openNode(key: string) {
    const a = graph.getNodeAttributes(key);
    // The graph keeps its tab; notes open in a new one.
    if (a.kind === "note") app.openNote(a.path as string, { newTab: true });
    else app.openLink(a.path as string, null, true);
  }

  function pick(key: string, q: string | null = null) {
    picked = key;
    pickedFor = q;
    focusNode(key);
  }

  function findNode() {
    const q = query.trim().toLowerCase();
    if (!q) return;
    // Enter again on the same text opens the note it found.
    if (picked && pickedFor === q && graph.hasNode(picked)) return openNode(picked);
    let best: string | null = null;
    graph.forEachNode((k, a) => {
      if (!best && (a.label as string).toLowerCase().includes(q)) best = k;
    });
    if (best) pick(best, q);
    else app.toast(`No note named like "${query}".`);
  }

  /** On the focused graph: arrow keys pick the next or previous note by name, Enter opens it. */
  function onCanvasKey(e: KeyboardEvent) {
    if (e.key === "Enter" && picked && graph.hasNode(picked)) {
      e.preventDefault();
      openNode(picked);
      return;
    }
    const step = ({ ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 } as Record<string, number>)[e.key];
    if (!step || graph.order === 0) return;
    e.preventDefault();
    const keys = graph.nodes().sort((a, b) => String(graph.getNodeAttribute(a, "label")).localeCompare(String(graph.getNodeAttribute(b, "label"))));
    const i = picked ? keys.indexOf(picked) : -1;
    pick(keys[i < 0 ? (step > 0 ? 0 : keys.length - 1) : (i + step + keys.length) % keys.length]);
  }

  onMount(() => {
    renderer = new Sigma(graph, container, {
      renderEdgeLabels: false,
      labelRenderedSizeThreshold: 7,
      labelColor: { color: colors().label },
      labelFont: getComputedStyle(document.body).fontFamily,
      labelSize: 12,
      defaultEdgeType: "line",
      zIndex: true,
      nodeReducer: (node, data) => {
        const activePath = app.active?.kind === "note" ? app.active.path : null;
        const res = { ...data };
        if (data.path === activePath) {
          res.color = theme.active;
          res.forceLabel = true;
          res.zIndex = 2;
        }
        if (hovered) {
          if (near.has(node)) {
            res.forceLabel = true;
            res.zIndex = 1;
            if (node === hovered) res.color = theme.active;
          } else {
            res.color = theme.dim;
            res.label = "";
          }
        }
        return res;
      },
      edgeReducer: (edge, data) => {
        if (!hovered) return data;
        if (nearEdges.has(edge)) return { ...data, color: theme.active, size: 1.2, zIndex: 1 };
        return { ...data, hidden: true };
      },
    });
    renderer.on("enterNode", ({ node }) => {
      hover(node);
      container.style.cursor = "pointer";
      renderer?.refresh({ skipIndexation: true });
    });
    renderer.on("leaveNode", () => {
      hover(null);
      container.style.cursor = "";
      renderer?.refresh({ skipIndexation: true });
    });
    renderer.on("clickNode", ({ node }) => openNode(node));
    // The nodes are pixels: the top canvas takes focus and the arrow keys.
    const mouse = renderer.getCanvases().mouse;
    mouse.tabIndex = 0;
    mouse.setAttribute("role", "application");
    mouse.setAttribute("aria-label", "Graph. Arrow keys pick a note, Enter opens it.");
    mouse.addEventListener("keydown", onCanvasKey);
    // Expose for tests.
    Object.assign(container, { __graph: graph, __sigma: renderer });
    // The theme is set by attributes of the html element and the accent by its
    // inline style, snippets are style elements in the head, and "System"
    // follows the media query.
    const observer = new MutationObserver(scheduleRecolor);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-light-theme", "data-dark-theme", "style"] });
    observer.observe(document.head, { childList: true });
    const scheme = matchMedia("(prefers-color-scheme: dark)");
    scheme.addEventListener("change", scheduleRecolor);
    void load();
    return () => {
      observer.disconnect();
      scheme.removeEventListener("change", scheduleRecolor);
      clearTimeout(recolorTimer);
      layout?.kill();
      clearTimeout(stopTimer);
      renderer?.kill();
      renderer = null;
    };
  });

  // Reload when notes or links change, or the setting flips.
  let firstRun = true;
  $effect(() => {
    void app.changeSeq;
    void settings.value.graphShowUnresolved;
    if (firstRun) {
      firstRun = false;
      return;
    }
    void load();
  });
</script>

<div class="graph-view" data-testid="graph-view">
  <div class="canvas" bind:this={container}></div>
  <div class="toolbar">
    <form
      onsubmit={(e) => {
        e.preventDefault();
        findNode();
      }}
    >
      <input class="text-input" placeholder="Find note in graph" aria-label="Find note in graph" bind:value={query} />
    </form>
    <button class="icon-btn" title={running ? "Stop layout" : "Run layout"} onclick={() => (running ? (layout?.stop(), (running = false)) : runLayout(6))}>
      <Icon name={running ? "x" : "refresh"} />
    </button>
    <label class="check" title="Show links to notes that do not exist yet">
      <input
        type="checkbox"
        checked={settings.value.graphShowUnresolved}
        onchange={(e) => settings.update({ graphShowUnresolved: e.currentTarget.checked })}
      />
      Unresolved
    </label>
  </div>
  <div class="sr-only" aria-live="polite">{picked && graph.hasNode(picked) ? graph.getNodeAttribute(picked, "label") : ""}</div>
  <div class="stats muted" data-testid="graph-stats">
    {#if loading}Loading…{:else if error}{error}{:else}{stats.nodes.toLocaleString()} notes · {stats.edges.toLocaleString()} links{running ? " · laying out" : ""}{/if}
  </div>
</div>

<style>
  .graph-view {
    position: relative;
    height: 100%;
    width: 100%;
    background: var(--bg);
  }
  .canvas {
    position: absolute;
    inset: 0;
  }
  .canvas :global(canvas.sigma-mouse:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .toolbar {
    position: absolute;
    top: 12px;
    left: 12px;
    display: flex;
    gap: 6px;
    align-items: center;
    background: color-mix(in srgb, var(--bg) 85%, transparent);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 6px;
    box-shadow: var(--shadow);
  }
  .toolbar input.text-input {
    width: 200px;
    padding: 5px 8px;
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: center;
    font-size: 12.5px;
    color: var(--text-muted);
    padding: 0 6px;
  }
  .check input {
    accent-color: var(--accent);
  }
  .stats {
    position: absolute;
    bottom: 10px;
    right: 14px;
    font-size: 12px;
  }
</style>
