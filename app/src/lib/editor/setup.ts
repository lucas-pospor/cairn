// CodeMirror 6 configuration for notes.

import { EditorState, StateEffect, Annotation, Transaction, type Extension, type TransactionSpec } from "@codemirror/state";
import {
  EditorView,
  Decoration,
  type DecorationSet,
  ViewPlugin,
  type ViewUpdate,
  MatchDecorator,
  keymap,
  drawSelection,
  dropCursor,
  highlightSpecialChars,
  rectangularSelection,
  placeholder,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab, isolateHistory } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { yamlFrontmatter } from "@codemirror/lang-yaml";
import { livePreview, livePreviewCompartment } from "./livePreview";
import { settings } from "../settings.svelte";
import { languages } from "@codemirror/language-data";
import {
  syntaxHighlighting,
  HighlightStyle,
  indentOnInput,
  bracketMatching,
  syntaxTree,
} from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  type Completion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import { search, searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import fuzzysort from "fuzzysort";
import { openWikilinkQuery, trimLink, wikilinkAt, type LinkIndex } from "../links";
import { displayName, parent } from "../paths";
import { isMobile } from "../platform";
import type { LinkKind } from "../types";
import { resetLineBreaks } from "./lineBreaks";
import { textChanges } from "./textChanges";

export interface EditorHooks {
  linkIndex(): LinkIndex | null;
  /** All file paths in the vault, for autocomplete. */
  files(): string[];
  openLink(target: string, subpath: string | null, newTab: boolean, kind?: LinkKind): void;
  openUrl(url: string): void;
  docChanged(view: EditorView): void;
  /** The selection changed, and it or the one before was not empty (the status bar counts it). */
  selectionChanged(): void;
  /** Vault path of the note in the editor. */
  notePath(): string;
  /** Title of the open note: the editor's accessible name. */
  noteTitle(): string;
  /** Store pasted or dropped files; returns Markdown linking to them. */
  saveFiles(files: File[]): Promise<string>;
}

/** Marks transactions that load content from disk (not user edits). */
export const fromDisk = Annotation.define<boolean>();

/**
 * A transaction that turns the document into `content` from disk. Only the
 * parts that differ are replaced, so the selection and the undo history map
 * through it. A reload is not an undo step (Ctrl+Z must not revert a change
 * another program, sync or git made to the file) unless `undoable`: then it
 * is a step of its own, as when the user chose to load the disk version.
 */
export function diskChange(state: EditorState, content: string, undoable = false): TransactionSpec {
  // Line breaks count as one character in both strings, as in document positions.
  return {
    changes: textChanges(state.doc.toString(), state.toText(content).toString()),
    effects: resetLineBreaks(content),
    annotations: [fromDisk.of(true), undoable ? isolateHistory.of("full") : Transaction.addToHistory.of(false)],
  };
}
/** Dispatched when the set of files changes so link styling updates. */
export const refreshLinks = StateEffect.define<null>();

const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
export const modKey = (e: { ctrlKey: boolean; metaKey: boolean }) => (isMac ? e.metaKey : e.ctrlKey);

function inCode(view: EditorView, pos: number): boolean {
  let node: ReturnType<ReturnType<typeof syntaxTree>["resolveInner"]> | null = syntaxTree(view.state).resolveInner(pos, 1);
  for (; node; node = node.parent) {
    if (/Code|CodeText|CodeBlock|FencedCode|InlineCode/.test(node.name)) return true;
  }
  return false;
}

function wikilinkDecorations(hooks: EditorHooks): Extension {
  const matcher = new MatchDecorator({
    regexp: /(!?)\[\[([^[\]\n]+?)\]\]/g,
    decorate: (add, from, to, match, view) => {
      if (inCode(view, from)) return;
      const inner = match[2];
      const target = trimLink(inner.split("|")[0].split("#")[0].replace(/\\$/, ""));
      const idx = hooks.linkIndex();
      const ok = !target || !idx || idx.exists(target, hooks.notePath());
      add(
        from,
        to,
        Decoration.mark({
          class: ok ? "cm-wikilink" : "cm-wikilink cm-wikilink-unresolved",
          attributes: { title: `${isMac ? "⌘" : "Ctrl"}+click to open` },
        }),
      );
    },
  });
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = matcher.createDeco(view);
      }
      update(u: ViewUpdate) {
        if (u.transactions.some((tr) => tr.effects.some((e) => e.is(refreshLinks)))) {
          this.decorations = matcher.createDeco(u.view);
        } else {
          this.decorations = matcher.updateDeco(u, this.decorations);
        }
      }
    },
    { decorations: (v) => v.decorations },
  );
}

/** Monospace background for fenced and indented code block lines. */
function codeBlockLines(): Extension {
  const line = Decoration.line({ class: "cm-codeblock" });
  const build = (view: EditorView): DecorationSet => {
    const ranges: ReturnType<typeof line.range>[] = [];
    for (const { from, to } of view.visibleRanges) {
      syntaxTree(view.state).iterate({
        from,
        to,
        enter: (node) => {
          if (node.name !== "FencedCode" && node.name !== "CodeBlock") return;
          const first = view.state.doc.lineAt(node.from).number;
          const last = view.state.doc.lineAt(node.to).number;
          for (let n = first; n <= last; n++) ranges.push(line.range(view.state.doc.line(n).from));
          return false;
        },
      });
    }
    return Decoration.set(ranges, true);
  };
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = build(view);
      }
      update(u: ViewUpdate) {
        if (u.docChanged || u.viewportChanged || syntaxTree(u.startState) !== syntaxTree(u.state)) {
          this.decorations = build(u.view);
        }
      }
    },
    { decorations: (v) => v.decorations },
  );
}

/**
 * Keep the cursor line in view when the editor gets shorter while it has
 * focus, as when the on-screen keyboard opens or the phone turns to landscape.
 */
function keepCursorInView(): Extension {
  return ViewPlugin.fromClass(
    class {
      height = 0;
      observer: ResizeObserver;
      constructor(view: EditorView) {
        this.observer = new ResizeObserver(([entry]) => {
          const h = entry.contentRect.height;
          if (h < this.height && view.hasFocus) view.dispatch({ effects: EditorView.scrollIntoView(view.state.selection.main.head) });
          this.height = h;
        });
        this.observer.observe(view.scrollDOM);
      }
      destroy() {
        this.observer.disconnect();
      }
    },
  );
}

/** Find a link (wiki, markdown or bare URL) at a document position. */
export function linkAtPos(
  state: EditorState,
  pos: number,
): { kind: LinkKind; target: string; subpath: string | null } | { kind: "url"; url: string } | null {
  const line = state.doc.lineAt(pos);
  const rel = pos - line.from;
  const wl = wikilinkAt(line.text, rel);
  if (wl && rel > wl.from && rel < wl.to) return { kind: "wiki", target: wl.target, subpath: wl.subpath };
  const mdRe = /!?\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g;
  let m: RegExpExecArray | null;
  while ((m = mdRe.exec(line.text))) {
    if (rel >= m.index && rel <= m.index + m[0].length) {
      const url = m[1].replace(/^<|>$/g, "");
      if (/^[a-z][a-z0-9+.-]+:/i.test(url)) return { kind: "url", url };
      const [path, sub] = url.split("#");
      return { kind: "markdown", target: decodeURIComponent(path), subpath: sub ? decodeURIComponent(sub) : null };
    }
  }
  const urlRe = /\bhttps?:\/\/[^\s<>()]+/g;
  while ((m = urlRe.exec(line.text))) {
    if (rel >= m.index && rel <= m.index + m[0].length) return { kind: "url", url: m[0] };
  }
  return null;
}

/**
 * The link at the cursor, for following it from the keyboard. The cursor
 * just after a link (as after typing its "]]") counts too.
 */
export function linkAtCursor(state: EditorState): ReturnType<typeof linkAtPos> {
  const head = state.selection.main.head;
  return linkAtPos(state, head) ?? (head > state.doc.lineAt(head).from ? linkAtPos(state, head - 1) : null);
}

function fileDrops(hooks: EditorHooks): Extension {
  const insert = async (view: EditorView, files: File[], pos: number) => {
    const text = await hooks.saveFiles(files);
    if (!text) return;
    const at = Math.min(pos, view.state.doc.length);
    view.dispatch({ changes: { from: at, insert: text }, selection: { anchor: at + text.length }, userEvent: "input.paste" });
  };
  return EditorView.domEventHandlers({
    paste(e, view) {
      const files = [...(e.clipboardData?.files ?? [])];
      if (!files.length) return false;
      e.preventDefault();
      void insert(view, files, view.state.selection.main.head);
      return true;
    },
    drop(e, view) {
      const files = [...(e.dataTransfer?.files ?? [])];
      if (!files.length) return false;
      e.preventDefault();
      const pos = view.posAtCoords({ x: e.clientX, y: e.clientY }) ?? view.state.selection.main.head;
      void insert(view, files, pos);
      return true;
    },
  });
}

function linkClicks(hooks: EditorHooks): Extension {
  return EditorView.domEventHandlers({
    mousedown(e, view) {
      if (e.button !== 0 && e.button !== 1) return false;
      if (!modKey(e) && e.button !== 1) return false;
      const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
      if (pos == null) return false;
      const link = linkAtPos(view.state, pos);
      if (!link) return false;
      e.preventDefault();
      // Ctrl/Cmd+click and middle-click open a new tab, as on a rendered link.
      if (link.kind === "url") hooks.openUrl(link.url);
      else hooks.openLink(link.target, link.subpath, true, link.kind);
      return true;
    },
  });
}

// Cached per file list so typing does not rebuild 10k objects per keystroke.
let cachedFiles: string[] | null = null;
let cachedTargets: { path: string; name: string }[] = [];
function completionTargets(files: string[]) {
  if (files !== cachedFiles) {
    cachedFiles = files;
    cachedTargets = files.map((p) => ({ path: p, name: displayName(p) }));
  }
  return cachedTargets;
}

function wikilinkCompletion(hooks: EditorHooks) {
  return (ctx: CompletionContext): CompletionResult | null => {
    const line = ctx.state.doc.lineAt(ctx.pos);
    const q = openWikilinkQuery(line.text.slice(0, ctx.pos - line.from));
    if (!q) return null;
    const from = line.from + q.start;
    const idx = hooks.linkIndex();
    const targets = completionTargets(hooks.files());
    const picked = q.query
      ? fuzzysort.go(q.query, targets, { keys: ["name", "path"], limit: 50 }).map((r) => r.obj)
      : [...targets].sort((a, b) => a.name.localeCompare(b.name)).slice(0, 50);
    const options: Completion[] = picked.flatMap((f) => {
      const text = idx ? idx.linkText(f.path) : f.name;
      if (text === null) return []; // no wikilink reaches this file
      return {
        label: f.name,
        detail: parent(f.path) || undefined,
        type: /\.(md|markdown)$/i.test(f.path) ? "text" : "variable",
        apply: (view, _c, aFrom, aTo) => {
          const after = view.state.sliceDoc(aTo, aTo + 2);
          const closing = after === "]]" ? "" : "]]";
          const end = aFrom + text.length + 2;
          view.dispatch({
            changes: { from: aFrom, to: aTo, insert: text + closing },
            selection: { anchor: end },
            userEvent: "input.complete",
          });
        },
      };
    });
    return { from, options, filter: false };
  };
}

const highlight = HighlightStyle.define([
  { tag: t.heading1, class: "md-h md-h1" },
  { tag: t.heading2, class: "md-h md-h2" },
  { tag: t.heading3, class: "md-h md-h3" },
  { tag: [t.heading4, t.heading5, t.heading6], class: "md-h md-h4" },
  { tag: t.strong, class: "md-strong" },
  { tag: t.emphasis, class: "md-em" },
  { tag: t.strikethrough, class: "md-strike" },
  { tag: t.link, class: "md-link" },
  { tag: t.url, class: "md-url" },
  { tag: t.monospace, class: "md-code" },
  { tag: t.quote, class: "md-quote" },
  { tag: [t.processingInstruction, t.meta], class: "md-mark" },
  { tag: t.contentSeparator, class: "md-hr" },
  { tag: t.list, class: "md-list" },
  // code block contents (from language-data grammars)
  { tag: t.keyword, class: "tok-keyword" },
  { tag: [t.string, t.special(t.string)], class: "tok-string" },
  { tag: [t.number, t.bool, t.null], class: "tok-number" },
  { tag: t.comment, class: "tok-comment" },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], class: "tok-fn" },
  { tag: [t.typeName, t.className], class: "tok-type" },
  { tag: t.propertyName, class: "tok-prop" },
]);

const baseTheme = EditorView.theme({
  "&": { height: "100%" },
  ".cm-scroller": { fontFamily: "var(--font-text)", lineHeight: "1.65", overflow: "auto" },
  ".cm-content": {
    maxWidth: "var(--line-width)",
    margin: "0 auto",
    padding: "28px 32px 40vh",
    caretColor: "var(--accent)",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-cursor": { borderLeftColor: "var(--accent)", borderLeftWidth: "2px" },
  // CodeMirror's own #888 is too faint on the light theme's --bg.
  ".cm-placeholder": { color: "var(--text-faint)" },
});

export function livePreviewExtension(hooks: EditorHooks): Extension {
  return livePreview({
    linkIndex: hooks.linkIndex,
    notePath: hooks.notePath,
    openLink: hooks.openLink,
    openUrl: hooks.openUrl,
  });
}

export function noteExtensions(hooks: EditorHooks): Extension[] {
  return [
    highlightSpecialChars(),
    history(),
    drawSelection(),
    dropCursor(),
    EditorState.allowMultipleSelections.of(true),
    indentOnInput(),
    bracketMatching(),
    closeBrackets(),
    rectangularSelection(),
    highlightSelectionMatches(),
    search({ top: true }),
    EditorView.lineWrapping,
    yamlFrontmatter({ content: markdown({ base: markdownLanguage, codeLanguages: languages, addKeymap: true }) }),
    livePreviewCompartment.of([]),
    syntaxHighlighting(highlight),
    autocompletion({ override: [wikilinkCompletion(hooks)], icons: false, activateOnTyping: true }),
    keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap, ...completionKeymap, indentWithTab]),
    wikilinkDecorations(hooks),
    codeBlockLines(),
    linkClicks(hooks),
    fileDrops(hooks),
    placeholder("Start writing…"),
    // Recomputed on every update, so the name follows the open note.
    EditorView.contentAttributes.of(() => ({
      spellcheck: settings.value.spellcheck ? "true" : "false",
      autocorrect: "off",
      "aria-label": hooks.noteTitle() || "Note",
    })),
    baseTheme,
    isMobile ? keepCursorInView() : [],
    EditorView.updateListener.of((u) => {
      if (u.docChanged && !u.transactions.every((tr) => tr.annotation(fromDisk))) hooks.docChanged(u.view);
      if (u.selectionSet && [u.state, u.startState].some((s) => s.selection.ranges.some((r) => !r.empty))) hooks.selectionChanged();
    }),
  ];
}
