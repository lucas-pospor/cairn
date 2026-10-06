<script lang="ts">
  // Formatting buttons above the on-screen keyboard (touch devices).
  import { undo, redo, indentMore, indentLess } from "@codemirror/commands";
  import { app } from "../app.svelte";
  import { commands } from "../commands";
  import { toggleWrap, toggleTask, insertWikilink, cycleHeading } from "../editor/format";
  import { pluginOn } from "../corePlugins/core";
  import { templates } from "../corePlugins/templates";
  import Icon from "./Icon.svelte";

  type Action = { label: string; title: string; run: () => void; icon?: string; id?: string };

  function withView(fn: (v: NonNullable<typeof app.view>) => unknown) {
    return () => {
      const v = app.view;
      if (!v) return;
      fn(v);
      v.focus();
    };
  }

  // Shown while the Templates core plugin is on; it asks which template, then inserts it.
  const template: Action = { label: "", icon: "template", title: "Insert template", run: () => commands.run("templates:insert"), id: "toolbar-template" };

  const actions: Action[] = $derived([
    { label: "↶", title: "Undo", run: withView((v) => undo(v)) },
    { label: "↷", title: "Redo", run: withView((v) => redo(v)) },
    { label: "H", title: "Heading", run: withView(cycleHeading) },
    { label: "B", title: "Bold", run: withView((v) => toggleWrap(v, "**")) },
    { label: "I", title: "Italic", run: withView((v) => toggleWrap(v, "*")) },
    { label: "[[ ]]", title: "Link", run: withView(insertWikilink) },
    ...(pluginOn(templates) ? [template] : []),
    { label: "☐", title: "Checkbox", run: withView(toggleTask) },
    { label: "`", title: "Code", run: withView((v) => toggleWrap(v, "`")) },
    { label: "⇤", title: "Outdent", run: withView((v) => indentLess(v)) },
    { label: "⇥", title: "Indent", run: withView((v) => indentMore(v)) },
  ]);
</script>

<div class="toolbar" role="toolbar" aria-label="Formatting">
  {#each actions as a}
    <!-- mousedown/pointerdown default would blur the editor and close the keyboard -->
    <button title={a.title} aria-label={a.title} onpointerdown={(e) => e.preventDefault()} onclick={a.run} data-testid={a.id}
      >{#if a.icon}<Icon name={a.icon} size={18} />{:else}{a.label}{/if}</button
    >
  {/each}
</div>

<style>
  .toolbar {
    display: flex;
    gap: 2px;
    overflow-x: auto;
    padding: 4px 6px;
    border-top: 1px solid var(--border);
    background: var(--bg-side);
    scrollbar-width: none;
  }
  button {
    min-width: 44px;
    height: 40px;
    border-radius: 8px;
    font-weight: 600;
    color: var(--text-muted);
    flex: none;
  }
  button:active {
    background: var(--accent-soft);
  }
  button :global(svg) {
    display: block;
    margin: auto;
  }
</style>
