// Live Preview: Markdown renders inline while you edit. Syntax characters
// are hidden on every line except the ones holding the cursor, and some
// constructs (images, embeds, tables, frontmatter, rules, checkboxes) are
// replaced by widgets. The text in the file is never changed by rendering.

import {
  ChangeSet,
  Compartment,
  EditorSelection,
  type EditorState,
  type Extension,
  type Range,
  RangeSet,
  type SelectionRange,
  StateEffect,
  StateField,
  type Text,
  type Transaction,
  findClusterBreak,
} from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { syntaxTree } from "@codemirror/language";
import type { SyntaxNode, Tree } from "@lezer/common";
import { vaultUrl } from "../backend";
import { fillEmbeds } from "../embeds";
import { trimLink, type LinkIndex } from "../links";
import { FRONTMATTER_RE, renderMarkdownSync } from "../markdown";
import { AUDIO_EXTS, IMAGE_EXTS, VIDEO_EXTS, extension, parent, resolveRelative } from "../paths";
import type { LinkKind } from "../types";

export interface LivePreviewHooks {
  linkIndex(): LinkIndex | null;
  /** Vault path of the note shown in the editor. */
  notePath(): string;
  openLink(target: string, subpath: string | null, newTab: boolean, kind?: LinkKind): void;
  openUrl(url: string): void;
}

export const livePreviewCompartment = new Compartment();

const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

// ---------- helpers ----------

/** Line numbers that contain a selection (cursor) when the editor has focus. */
function activeLines(state: EditorState, focused: boolean): Set<number> {
  const out = new Set<number>();
  if (!focused) return out;
  for (const r of state.selection.ranges) {
    const a = state.doc.lineAt(r.from).number;
    const b = state.doc.lineAt(r.to).number;
    for (let n = a; n <= b; n++) out.add(n);
  }
  return out;
}

function rangeTouchesLines(state: EditorState, from: number, to: number, lines: Set<number>): boolean {
  const a = state.doc.lineAt(from).number;
  const b = state.doc.lineAt(to).number;
  for (let n = a; n <= b; n++) if (lines.has(n)) return true;
  return false;
}

function inCode(node: SyntaxNode | null): boolean {
  for (; node; node = node.parent) {
    if (/^(InlineCode|FencedCode|CodeBlock|CodeText|HTMLBlock|Comment)/.test(node.name)) return true;
  }
  return false;
}

function resolveImage(src: string, hooks: LivePreviewHooks): string {
  // As in the core, the part after '#' is not part of the path.
  let rel = src.split("#")[0];
  if (!rel || /^[a-z][a-z0-9+.-]*:/i.test(src) || src.startsWith("//")) return src;
  try {
    rel = decodeURIComponent(rel);
  } catch {}
  const idx = hooks.linkIndex();
  const note = hooks.notePath();
  const direct = resolveRelative(parent(note), rel);
  if (direct && idx?.has(direct)) return vaultUrl(direct);
  const viaName = idx?.resolve(rel, note);
  return vaultUrl(viaName ?? direct ?? rel);
}

/**
 * A left click on a block widget, outside its links and media controls, puts
 * the cursor on its first line (or `skip` lines below), which shows the
 * source. The position is read from the DOM at click time: the widget may have
 * moved since it was made.
 */
function revealOnMousedown(dom: HTMLElement, view: EditorView, skip = 0) {
  dom.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest("a, audio, video")) return;
    e.preventDefault();
    const doc = view.state.doc;
    const first = doc.lineAt(view.posAtDOM(dom)).number;
    view.dispatch({ selection: { anchor: doc.line(Math.min(first + skip, doc.lines)).from } });
    view.focus();
  });
}

// ---------- widgets ----------

class BulletWidget extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const s = document.createElement("span");
    s.className = "cm-lp-bullet";
    s.textContent = "•";
    return s;
  }
}

class CheckboxWidget extends WidgetType {
  constructor(readonly checked: boolean) {
    super();
  }
  eq(o: CheckboxWidget) {
    return o.checked === this.checked;
  }
  toDOM() {
    const box = document.createElement("input");
    box.type = "checkbox";
    box.className = "cm-lp-task";
    box.checked = this.checked;
    box.setAttribute("aria-label", this.checked ? "Done" : "To do");
    return box;
  }
  ignoreEvent() {
    return false;
  }
}

class HrWidget extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const d = document.createElement("span");
    d.className = "cm-lp-hr";
    return d;
  }
  ignoreEvent() {
    // A click puts the cursor on the rule's line.
    return false;
  }
}

class ImageWidget extends WidgetType {
  constructor(
    readonly src: string,
    readonly alt: string,
    readonly width: string | null,
    readonly block: boolean,
  ) {
    super();
  }
  eq(o: ImageWidget) {
    return o.src === this.src && o.alt === this.alt && o.width === this.width && o.block === this.block;
  }
  toDOM() {
    const wrap = document.createElement(this.block ? "div" : "span");
    wrap.className = this.block ? "cm-lp-image-block" : "cm-lp-image";
    const ext = extension(this.src.split("?")[0]);
    let el: HTMLElement;
    if (AUDIO_EXTS.has(ext)) {
      el = Object.assign(document.createElement("audio"), { controls: true, src: this.src });
    } else if (VIDEO_EXTS.has(ext)) {
      el = Object.assign(document.createElement("video"), { controls: true, src: this.src });
    } else {
      const img = document.createElement("img");
      img.src = this.src;
      img.alt = this.alt;
      if (this.width) img.width = Number(this.width);
      img.onerror = () => {
        wrap.classList.add("is-broken");
        wrap.textContent = `Image not found: ${this.alt || this.src}`;
      };
      el = img;
    }
    wrap.appendChild(el);
    return wrap;
  }
  get estimatedHeight() {
    return this.block ? 200 : -1;
  }
  ignoreEvent(e: Event) {
    // A click on an image puts the cursor on its line; audio and video keep
    // their controls.
    return e.target instanceof HTMLMediaElement;
  }
}

/** Fills note embeds again: dispatched when files in the vault change. */
export const refreshEmbeds = StateEffect.define<null>();

/** Counts `refreshEmbeds`; an embed made for an older count is filled again. */
const embedVersion = StateField.define<number>({
  create: () => 0,
  update: (v, tr) => (tr.effects.some((e) => e.is(refreshEmbeds)) ? v + 1 : v),
});

class EmbedWidget extends WidgetType {
  constructor(
    readonly target: string,
    readonly subpath: string,
    readonly source: string,
    readonly hooks: LivePreviewHooks,
    readonly version: number,
  ) {
    super();
  }
  get key() {
    return JSON.stringify([this.source, this.target, this.subpath]);
  }
  eq(o: EmbedWidget) {
    return o.key === this.key && o.version === this.version;
  }
  private placeholder() {
    const span = document.createElement("span");
    span.className = "embed";
    span.dataset.target = this.target;
    span.dataset.subpath = this.subpath;
    span.textContent = "…";
    return span;
  }
  toDOM(view: EditorView) {
    const wrap = document.createElement("div");
    wrap.className = "cm-lp-embed md-render";
    wrap.dataset.embed = this.key;
    wrap.appendChild(this.placeholder());
    void fillEmbeds(wrap, this.source, this.hooks.linkIndex());
    revealOnMousedown(wrap, view);
    return wrap;
  }
  updateDOM(dom: HTMLElement, view: EditorView) {
    // The same embed after a change in the vault: fill it again, and swap the
    // new content in once it is ready, so the old one stays until then.
    if (dom.dataset.embed !== this.key) return false;
    const gen = (dom.dataset.gen = String(Number(dom.dataset.gen ?? 0) + 1));
    const next = document.createElement("div");
    next.appendChild(this.placeholder());
    void fillEmbeds(next, this.source, this.hooks.linkIndex()).then(() => {
      if (dom.dataset.gen !== gen) return;
      dom.replaceChildren(...next.childNodes);
      view.requestMeasure();
    });
    return true;
  }
  get estimatedHeight() {
    return 120;
  }
  ignoreEvent(e: Event) {
    // Let clicks on links inside the embed reach our handler; the listener
    // added in toDOM handles the rest.
    return !(e.target instanceof HTMLElement && e.target.closest("a"));
  }
}

class HtmlBlockWidget extends WidgetType {
  constructor(
    readonly html: string,
    readonly className: string,
    /** A click puts the cursor this many lines below the block's first line. */
    readonly skip: number,
  ) {
    super();
  }
  eq(o: HtmlBlockWidget) {
    return o.html === this.html && o.className === this.className && o.skip === this.skip;
  }
  toDOM(view: EditorView) {
    const d = document.createElement("div");
    d.className = `${this.className} md-render`;
    d.innerHTML = this.html;
    revealOnMousedown(d, view, this.skip);
    return d;
  }
  ignoreEvent(e: Event) {
    // Let events on links (in tables) reach the clicks() handler; the
    // listener above handles the rest.
    return !(e.target instanceof HTMLElement && e.target.closest("a"));
  }
}

// ---------- inline decorations (visible ranges only) ----------

const hide = Decoration.replace({});
const WIKI_RE = /(!?)\[\[([^[\]\n]+?)\]\]/g;
// Same rule as TAG_RE in the core's parse.rs and the preview's tagPlugin:
// after whitespace, ',' or ';' (\p{M}: combining marks).
const TAG_RE = /(^|[\s,;])#([\p{L}\p{M}\p{N}_/-]*[\p{L}_/-][\p{L}\p{M}\p{N}_/-]*)/gu;

function buildInline(view: EditorView, hooks: LivePreviewHooks): DecorationSet {
  const { state } = view;
  const active = activeLines(state, view.hasFocus);
  const decos: Range<Decoration>[] = [];
  const tree = syntaxTree(state);
  const lineActive = (pos: number) => active.has(state.doc.lineAt(pos).number);

  for (const { from, to } of view.visibleRanges) {
    tree.iterate({
      from,
      to,
      enter: (ref) => {
        const n = ref.node;
        switch (n.name) {
          case "FencedCode":
          case "CodeBlock":
          case "HTMLBlock":
          case "Table":
          case "Frontmatter":
            return false;
          case "Blockquote": {
            const a = state.doc.lineAt(n.from).number;
            const b = state.doc.lineAt(n.to).number;
            for (let i = a; i <= b; i++) decos.push(Decoration.line({ class: "cm-lp-quote" }).range(state.doc.line(i).from));
            return;
          }
          case "HeaderMark": {
            if (lineActive(n.from)) return;
            const line = state.doc.lineAt(n.from);
            // opening "## " or closing " ##"
            let end = n.to;
            if (n.from === line.from || /^\s*$/.test(state.sliceDoc(line.from, n.from))) {
              if (state.sliceDoc(end, end + 1) === " ") end++;
              decos.push(hide.range(n.from, end));
            } else {
              decos.push(hide.range(n.from, n.to));
            }
            return;
          }
          case "EmphasisMark":
          case "StrikethroughMark":
          case "Escape": {
            if (lineActive(n.from)) return;
            if (n.name === "Escape") decos.push(hide.range(n.from, n.from + 1));
            else decos.push(hide.range(n.from, n.to));
            return;
          }
          case "InlineCode": {
            if (lineActive(n.from)) return false;
            for (let c = n.firstChild; c; c = c.nextSibling) {
              if (c.name === "CodeMark") decos.push(hide.range(c.from, c.to));
            }
            return false;
          }
          case "QuoteMark": {
            if (lineActive(n.from)) return;
            let end = n.to;
            if (state.sliceDoc(end, end + 1) === " ") end++;
            decos.push(hide.range(n.from, end));
            return;
          }
          case "ListMark": {
            if (lineActive(n.from)) return;
            const item = n.parent;
            const list = item?.parent;
            const isTask = !!item?.getChild("Task");
            if (list?.name === "BulletList") {
              let end = n.to;
              if (isTask && state.sliceDoc(end, end + 1) === " ") {
                end++;
                decos.push(hide.range(n.from, end));
              } else {
                decos.push(Decoration.replace({ widget: new BulletWidget() }).range(n.from, n.to));
              }
            }
            return;
          }
          case "TaskMarker": {
            if (lineActive(n.from)) return;
            const checked = /x/i.test(state.sliceDoc(n.from, n.to));
            decos.push(Decoration.replace({ widget: new CheckboxWidget(checked) }).range(n.from, n.to));
            const line = state.doc.lineAt(n.from);
            if (checked) decos.push(Decoration.mark({ class: "cm-lp-task-done" }).range(n.to, line.to));
            return;
          }
          case "HorizontalRule": {
            if (lineActive(n.from)) return;
            decos.push(Decoration.replace({ widget: new HrWidget() }).range(n.from, n.to));
            return;
          }
          case "Image": {
            if (lineActive(n.from)) return false;
            const url = n.getChild("URL");
            if (!url) return false;
            const text = state.sliceDoc(n.from, n.to);
            const alt = /^!\[([^\]]*)\]/.exec(text)?.[1] ?? "";
            const line = state.doc.lineAt(n.from);
            if (state.sliceDoc(line.from, line.to).trim() === text.trim()) return false; // block field
            const src = resolveImage(state.sliceDoc(url.from, url.to).replace(/^<|>$/g, ""), hooks);
            decos.push(Decoration.replace({ widget: new ImageWidget(src, alt, null, false) }).range(n.from, n.to));
            return false;
          }
          case "Link": {
            if (lineActive(n.from)) return false;
            const marks = n.getChildren("LinkMark");
            const url = n.getChild("URL");
            if (marks.length < 2) return false;
            const open = marks[0];
            const close = marks[1];
            const href = url ? state.sliceDoc(url.from, url.to).replace(/^<|>$/g, "") : "";
            if (!href) return false; // reference links: leave as typed
            decos.push(hide.range(open.from, open.to));
            if (close.from > open.to) {
              decos.push(
                Decoration.mark({ class: "cm-lp-link", attributes: { "data-url": href, title: href } }).range(open.to, close.from),
              );
            }
            decos.push(hide.range(close.from, n.to));
            return false;
          }
          case "Autolink": {
            if (lineActive(n.from)) return false;
            const url = n.getChild("URL");
            if (!url) return false;
            decos.push(hide.range(n.from, url.from));
            decos.push(
              Decoration.mark({ class: "cm-lp-link", attributes: { "data-url": state.sliceDoc(url.from, url.to) } }).range(
                url.from,
                url.to,
              ),
            );
            decos.push(hide.range(url.to, n.to));
            return false;
          }
          case "URL": {
            // bare URLs (GFM autolinks); on the cursor line they are text to edit
            if (lineActive(n.from)) return;
            if (n.parent?.name === "Paragraph" || n.parent?.name === "Document") {
              decos.push(
                Decoration.mark({ class: "cm-lp-link", attributes: { "data-url": state.sliceDoc(n.from, n.to) } }).range(n.from, n.to),
              );
            }
            return;
          }
        }
      },
    });

    // Wikilinks and tags are not in the Markdown grammar; scan the text.
    const text = state.sliceDoc(from, to);
    WIKI_RE.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = WIKI_RE.exec(text))) {
      const start = from + m.index;
      const end = start + m[0].length;
      if (inCode(tree.resolveInner(start, 1))) continue;
      const embed = m[1] === "!";
      const line = state.doc.lineAt(start);
      if (lineActive(start)) continue;
      const inner = m[2];
      const bar = inner.indexOf("|");
      const main = (bar >= 0 ? inner.slice(0, bar) : inner).replace(/\\$/, "");
      const alias = bar >= 0 ? inner.slice(bar + 1) : null;
      const hashAt = main.indexOf("#");
      const target = trimLink(hashAt >= 0 ? main.slice(0, hashAt) : main);
      const sub = hashAt >= 0 ? trimLink(main.slice(hashAt + 1)) : "";
      if (embed) {
        if (state.sliceDoc(line.from, line.to).trim() === m[0]) continue; // block field
        const resolved = hooks.linkIndex()?.resolve(target, hooks.notePath());
        if (resolved && IMAGE_EXTS.has(extension(resolved))) {
          const width = alias && /^\d+(x\d+)?$/.test(alias) ? alias.split("x")[0] : null;
          decos.push(Decoration.replace({ widget: new ImageWidget(vaultUrl(resolved), target, width, false) }).range(start, end));
          continue;
        }
      }
      const ok = !target || (hooks.linkIndex()?.exists(target, hooks.notePath()) ?? true);
      const cls = `cm-lp-link cm-lp-wikilink${ok ? "" : " is-unresolved"}`;
      const attrs = { "data-wiki": target, "data-sub": sub, title: `${main}` };
      const textFrom = alias != null ? start + 2 + (embed ? 1 : 0) + bar + 1 : start + 2 + (embed ? 1 : 0);
      const textTo = end - 2;
      decos.push(hide.range(start, textFrom));
      if (textTo > textFrom) decos.push(Decoration.mark({ class: cls, attributes: attrs }).range(textFrom, textTo));
      decos.push(hide.range(textTo, end));
    }
    TAG_RE.lastIndex = 0;
    while ((m = TAG_RE.exec(text))) {
      const start = from + m.index + m[1].length;
      const end = start + 1 + m[2].length;
      if (inCode(tree.resolveInner(start, 1))) continue;
      decos.push(Decoration.mark({ class: "cm-lp-tag", attributes: { "data-tag": m[2] } }).range(start, end));
    }
  }
  return Decoration.set(decos, true);
}

function inlinePlugin(hooks: LivePreviewHooks): Extension {
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = buildInline(view, hooks);
      }
      update(u: ViewUpdate) {
        if (
          u.docChanged ||
          u.selectionSet ||
          u.viewportChanged ||
          u.focusChanged ||
          syntaxTree(u.startState) !== syntaxTree(u.state) ||
          u.transactions.some((t) => t.effects.length)
        ) {
          this.decorations = buildInline(u.view, hooks);
        }
      }
    },
    { decorations: (v) => v.decorations },
  );
}

// ---------- block widgets (state field: they may span lines) ----------

/** The frontmatter at the top of the note: where it ends, and its YAML. */
function frontmatter(doc: Text): { to: number; yaml: string } | null {
  const start = doc.sliceString(0, 4);
  if (!start.startsWith("---") && !start.startsWith("\uFEFF---")) return null;
  // The same rule as the core and the reading view (FRONTMATTER_RE).
  const fm = FRONTMATTER_RE.exec(doc.sliceString(0, Math.min(doc.length, 20000)));
  return fm ? { to: doc.lineAt(fm[0].trimEnd().length).to, yaml: fm[1] ?? "" } : null;
}

/** Nodes that can hold a table; the walks below do not look inside any other. */
const TABLE_PARENTS = new Set(["Document", "Blockquote", "BulletList", "OrderedList", "ListItem"]);

/** Call `f` for every table that touches from..to. */
function tablesIn(tree: Tree, from: number, to: number, f: (from: number, to: number) => void) {
  tree.iterate({
    from,
    to,
    enter: (n) => {
      if (n.name === "Table") {
        f(n.from, n.to);
        return false;
      }
      return TABLE_PARENTS.has(n.name);
    },
  });
}

/**
 * The block decorations for the whole lines from..to, added to `decos`: the
 * properties box (when `from` is 0), tables, and image and embed lines.
 */
function blocksIn(
  state: EditorState,
  tree: Tree,
  from: number,
  to: number,
  hooks: LivePreviewHooks,
  decos: Range<Decoration>[],
) {
  // The field cannot see focus; treat the selection as active always.
  const active = activeLines(state, true);
  const doc = state.doc;

  // Frontmatter as a properties box.
  const fm = from === 0 ? frontmatter(doc) : null;
  if (fm && !rangeTouchesLines(state, 0, fm.to, active)) {
    decos.push(
      Decoration.replace({ widget: new HtmlBlockWidget(propertiesHtml(fm.yaml), "cm-lp-props", 1), block: true }).range(0, fm.to),
    );
  }

  // Tables, and image/embed lines.
  tablesIn(tree, from, to, (a, b) => {
    const start = doc.lineAt(a).from;
    const end = doc.lineAt(b).to;
    if (start < from || end > to || rangeTouchesLines(state, a, b, active)) return;
    const html = renderMarkdownSync(doc.sliceString(start, end), { links: hooks.linkIndex(), sourcePath: hooks.notePath() });
    decos.push(Decoration.replace({ widget: new HtmlBlockWidget(html, "cm-lp-table", 0), block: true }).range(start, end));
  });

  for (let i = doc.lineAt(from).number, last = doc.lineAt(to).number; i <= last; i++) {
    const line = doc.line(i);
    if (active.has(i)) continue;
    const t = line.text.trim();
    if (!t.startsWith("![")) continue;
    if (inCode(tree.resolveInner(line.from + line.text.indexOf("!"), 1))) continue;
    const wiki = /^!\[\[([^[\]\n]+?)\]\]$/.exec(t);
    if (wiki) {
      const inner = wiki[1];
      const bar = inner.indexOf("|");
      const main = bar >= 0 ? inner.slice(0, bar) : inner;
      const alias = bar >= 0 ? inner.slice(bar + 1) : null;
      const hashAt = main.indexOf("#");
      const target = trimLink(hashAt >= 0 ? main.slice(0, hashAt) : main);
      const sub = hashAt >= 0 ? trimLink(main.slice(hashAt + 1)) : "";
      const resolved = hooks.linkIndex()?.resolve(target, hooks.notePath());
      const ext = extension(resolved ?? target);
      let widget: WidgetType;
      if (resolved && (IMAGE_EXTS.has(ext) || AUDIO_EXTS.has(ext) || VIDEO_EXTS.has(ext))) {
        const width = alias && /^\d+(x\d+)?$/.test(alias) ? alias.split("x")[0] : null;
        widget = new ImageWidget(vaultUrl(resolved), target, width, true);
      } else {
        widget = new EmbedWidget(target, sub, hooks.notePath(), hooks, state.field(embedVersion, false) ?? 0);
      }
      decos.push(Decoration.replace({ widget, block: true }).range(line.from, line.to));
      continue;
    }
    const img = /^!\[([^\]]*)\]\(<?([^)\s>]+)>?(?:\s+"[^"]*")?\)$/.exec(t);
    if (img) {
      const [alt, w] = img[1].split("|");
      const width = w && /^\d+$/.test(w) ? w : null;
      decos.push(
        Decoration.replace({ widget: new ImageWidget(resolveImage(img[2], hooks), alt, width, true), block: true }).range(
          line.from,
          line.to,
        ),
      );
    }
  }
}

/** All block decorations of the note. */
function buildBlocks(state: EditorState, hooks: LivePreviewHooks): DecorationSet {
  const decos: Range<Decoration>[] = [];
  blocksIn(state, syntaxTree(state), 0, state.doc.length, hooks, decos);
  return Decoration.set(decos, true);
}

/**
 * The Markdown document node, whose children are the note's top-level
 * blocks, or null when the tree has none yet. With frontmatter support the
 * Markdown document is mounted in the Body of an outer document, and a parse
 * that ran out of time before the Markdown parse started has only the outer
 * one (a Body without Markdown, or a frontmatter cut short).
 */
function markdownDoc(tree: Tree): SyntaxNode | null {
  const top = tree.topNode;
  const last = top.lastChild;
  if (last?.name === "Document") return last;
  if (last && (last.name === "Body" || last.name === "Frontmatter" || last.type.isError)) return null;
  return top;
}

/**
 * How far the blocks of a Markdown document are final: the end of the note
 * when the parse got there, else the start of the line of the last block
 * the parse reached (a parse that stops early closes the blocks that are
 * still open, and the next parse may make that one longer or different).
 * When the parse of the outer document stopped early, the Markdown parse
 * stopped there too, and what the Markdown tree has after that is not a
 * parse of the text: placeholders for older blocks it would have reused.
 */
function finalTo(md: SyntaxNode, tree: Tree, doc: Text): number {
  const parsed = Math.min(md.to, tree.length);
  if (parsed >= doc.length) return doc.length;
  return doc.lineAt(md.childBefore(parsed)?.from ?? md.from).from;
}

/**
 * Zero-length top-level blocks: placeholders that a parse which stopped
 * early (the outer parse at a set position, after an earlier parse ran out
 * of time) put where it would have reused older blocks reaching past the
 * stop. Later parses can keep one and parse the text after it as if the
 * block it stands for were empty, until they parse that text again (an
 * upstream quirk: a complete parse never makes them). Returns those that
 * start in `from` and the blocks after it, up to `to`.
 */
function placeholdersIn(from: SyntaxNode | null, to: number): number[] {
  const out: number[] = [];
  for (let n = from; n && n.from <= to; n = n.nextSibling) if (n.from === n.to) out.push(n.from);
  return out;
}

/** Whether `md` still has the placeholder at `pos`. */
function hasPlaceholder(md: SyntaxNode, pos: number): boolean {
  for (let n = md.childAfter(pos - 1); n && n.from <= pos; n = n.nextSibling) {
    if (n.from === pos && n.to === pos) return true;
  }
  return false;
}

/** The block decorations, and what they were made from. */
interface Blocks {
  decos: DecorationSet;
  /**
   * The syntax tree the decorations were last brought up to date with, its
   * document, and how far its blocks are final (-1: it had no Markdown).
   */
  tree: Tree;
  doc: Text;
  end: number;
  /** The changes made since then, waiting for a tree with Markdown in it. */
  changes: ChangeSet;
  /**
   * Decorations from here on may be out of date (the parse has not got there
   * since the note opened or an edit changed the blocks up to there); they
   * are made again as the parse gets there. The end of the note when all are
   * up to date.
   */
  good: number;
  /** Ranges to make again once the tree's blocks are final there. */
  dirty: [number, number][];
  /** Placeholders in the tree (see placeholdersIn). */
  placeholders: number[];
  /** Look for placeholders in the whole tree: a parse ran out of time. */
  scan: boolean;
  active: Set<number>;
  embeds: number;
  links: LinkIndex | null;
  path: string;
}

function blocksFor(state: EditorState, hooks: LivePreviewHooks): Blocks {
  const start: Blocks = {
    decos: Decoration.none,
    tree: syntaxTree(state),
    doc: state.doc,
    end: -1,
    changes: ChangeSet.empty(state.doc.length),
    good: 0,
    dirty: [],
    placeholders: [],
    scan: false,
    active: activeLines(state, true),
    embeds: state.field(embedVersion, false) ?? 0,
    links: hooks.linkIndex(),
    path: hooks.notePath(),
  };
  return syncBlocks(start, state, hooks);
}

/**
 * Where the block structure of the note can differ from the one the
 * decorations were made for, after `value.changes`: the top-level blocks
 * that held the changed lines in the old tree and hold them in the new one
 * (where a setext underline or a table's delimiter row also takes in the
 * line above), and the blocks after them up to the first one that both trees
 * have at the same place. From there on the text is the same, so it parses
 * the same. Only blocks that are final in their tree count. `open`: the new
 * tree's final blocks end before that place, so from `to` on the blocks are
 * not known yet.
 */
function changedBlocks(
  value: Blocks,
  doc: Text,
  md: SyntaxNode,
  end: number,
  good: number,
): { from: number; to: number; open: boolean } {
  const { changes, doc: docA } = value;
  let fromA = Infinity;
  let toA = 0;
  let fromB = Infinity;
  let toB = 0;
  changes.iterChangedRanges((fa, ta, fb, tb) => {
    fromA = Math.min(fromA, fa);
    toA = Math.max(toA, ta);
    fromB = Math.min(fromB, fb);
    toB = Math.max(toB, tb);
  });
  let from = doc.lineAt(fromB).from;
  let to = doc.lineAt(toB).to;
  const oldMd = value.end < 0 ? null : markdownDoc(value.tree);
  if (oldMd) {
    const last = docA.lineAt(toA).to;
    for (let n = oldMd.childAfter(docA.lineAt(fromA).from); n && n.from <= last; n = n.nextSibling) {
      from = Math.min(from, changes.mapPos(n.from, -1));
      to = Math.max(to, changes.mapPos(n.to, 1));
    }
  }
  const complete = end >= doc.length;
  const delta = doc.length - docA.length;
  for (let n = md.childAfter(from); n; n = n.nextSibling) {
    if (n.from > to) {
      // Decorations from `good` on are made again as the parse gets there.
      if (n.from >= good) return { from, to, open: false };
      const o = oldMd?.childAfter(n.from - delta);
      if (
        o &&
        o.name === n.name &&
        o.from === n.from - delta &&
        o.to === n.to - delta &&
        (value.end >= docA.length || o.from < value.end)
      )
        return { from, to, open: false };
    } else from = Math.min(from, n.from);
    if (!complete && n.from >= end) return { from, to: n.from > to ? to : from, open: true };
    to = Math.max(to, n.to);
  }
  return complete ? { from, to: doc.length, open: false } : { from, to: from, open: true };
}

/**
 * Update the block decorations for a transaction. Only the parts of the note
 * that can have changed are made again: the blocks around an edit, the lines
 * that hold or held a cursor, what the parser added, and embeds after a
 * change in the vault. Everything else is kept (mapped through the edit).
 */
function updateBlocks(value: Blocks, tr: Transaction, hooks: LivePreviewHooks): Blocks {
  const { state, changes } = tr;
  const doc = state.doc;
  const active = activeLines(state, true);
  const embeds = state.field(embedVersion, false) ?? 0;
  const links = hooks.linkIndex();
  const path = hooks.notePath();
  const activeChanged = active.size !== value.active.size || [...active].some((n) => !value.active.has(n));
  let { decos, good, dirty, placeholders } = value;
  if (tr.docChanged) {
    decos = decos.map(changes);
    good = good >= tr.startState.doc.length ? doc.length : changes.mapPos(good, -1);
    dirty = dirty.map(([a, b]) => [changes.mapPos(a, -1), changes.mapPos(b, 1)]);
    placeholders = placeholders.map((pos) => changes.mapPos(pos, -1));
  }
  // Tables and embeds show links as resolved or not: when the link index or
  // the note changes, all of them are out of date.
  if (links !== value.links || path !== value.path) good = 0;
  if (tr.docChanged || activeChanged) {
    dirty = [...dirty];
    const old = tr.startState;
    for (const r of old.selection.ranges) {
      dirty.push([changes.mapPos(old.doc.lineAt(r.from).from, -1), changes.mapPos(old.doc.lineAt(r.to).to, 1)]);
    }
    // No widget on a line with a cursor: take those off at once, also when
    // the tree is not far enough yet to make the lines again.
    for (const r of state.selection.ranges) {
      const a = doc.lineAt(r.from).from;
      const b = doc.lineAt(r.to).to;
      dirty.push([a, b]);
      decos = decos.update({ filter: (from, to) => from > b || to < a, filterFrom: a, filterTo: b });
    }
  }
  if (embeds !== value.embeds) {
    dirty = [...dirty];
    decos.between(0, doc.length, (from, to, d) => {
      if (d.spec.widget instanceof EmbedWidget) dirty.push([from, to]);
    });
  }
  const pending = tr.docChanged ? value.changes.compose(changes) : value.changes;
  return syncBlocks(
    { ...value, decos, changes: pending, good, dirty, placeholders, active, embeds, links, path },
    state,
    hooks,
  );
}

/**
 * Bring the block decorations up to date with the current syntax tree, as
 * far as its blocks are final: the changes since the last tree, what the
 * parse added, and the dirty ranges. A tree without Markdown in it (the
 * parse ran out of time before the Markdown parse started) changes nothing:
 * the decorations wait for the next one.
 */
function syncBlocks(value: Blocks, state: EditorState, hooks: LivePreviewHooks): Blocks {
  const doc = state.doc;
  const tree = syntaxTree(state);
  const md = markdownDoc(tree);
  // The parse ran out of time before the Markdown parse started.
  if (!md) return { ...value, scan: true };
  const end = finalTo(md, tree, doc);
  let { good, dirty } = value;
  // Where a placeholder comes or goes, the parse makes the blocks after it
  // again, also where nothing was edited: those are out of date. New ones
  // come after a parse ran out of time, at the stop of a later parse; look
  // at the whole tree until a complete one, else near the stop.
  let scan = value.scan || md.to < tree.length;
  let placeholders = value.placeholders;
  if (tree !== value.tree || value.end < 0) {
    placeholders = placeholders.filter((pos) => {
      if (pos >= end || hasPlaceholder(md, pos)) return true;
      good = Math.min(good, pos);
      return false;
    });
    const stop = Math.min(md.to, tree.length);
    for (const pos of placeholdersIn(scan ? md.firstChild : md.childBefore(stop), scan ? end : stop)) {
      if (placeholders.includes(pos)) continue;
      placeholders.push(pos);
      good = Math.min(good, pos);
    }
    if (end >= doc.length) scan = false;
  }
  if (!value.changes.empty) {
    const r = changedBlocks(value, doc, md, end, good);
    if (r.open && md.to < tree.length && r.to < good) {
      // The Markdown parse ran out of time before it got past the change,
      // and goes on in the background. Wait for it rather than make all the
      // rest of the note again (the dirty ranges can be made already).
      const { decos, left } = rebuildBlocks(state, tree, end, value.decos, dirty, hooks);
      return { ...value, decos, good, dirty: left, placeholders, scan };
    }
    dirty = [...dirty, [r.from, r.to]];
    if (r.open) good = Math.min(good, r.to);
  }
  // The parse got further (up to the line before `end`, when that is not
  // the end of the note).
  if (end > good) {
    dirty = [...dirty, [good, end < doc.length ? end - 1 : end]];
    good = end;
  }
  // What is out of date from `good` on is made when the parse gets there.
  if (good < doc.length) dirty = dirty.filter(([a]) => a < good);
  const { decos, left } = rebuildBlocks(state, tree, end, value.decos, dirty, hooks);
  return { ...value, decos, tree, doc, end, changes: ChangeSet.empty(doc.length), good, dirty: left, placeholders, scan };
}

/**
 * Make the block decorations in the `dirty` ranges again from `tree`, whose
 * blocks are final up to `end`. Each range first grows to whole blocks; one
 * that then reaches `end` waits for a later tree, and is returned in `left`.
 */
function rebuildBlocks(
  state: EditorState,
  tree: Tree,
  end: number,
  decos: DecorationSet,
  dirty: [number, number][],
  hooks: LivePreviewHooks,
): { decos: DecorationSet; left: [number, number][] } {
  if (!dirty.length) return { decos, left: dirty };
  const doc = state.doc;
  const fm = frontmatter(doc);
  const clamp = (pos: number) => Math.max(0, Math.min(doc.length, pos));
  let ranges: [number, number][] = dirty.map(([a, b]) => [doc.lineAt(clamp(a)).from, doc.lineAt(clamp(b)).to]);
  // Grow each range to whole blocks: the frontmatter, the decorations it
  // touches and the tables it touches, until that adds nothing.
  for (let grown = true; grown; ) {
    grown = false;
    ranges.sort((x, y) => x[0] - y[0]);
    const merged: [number, number][] = [];
    for (const r of ranges) {
      const last = merged[merged.length - 1];
      if (last && r[0] <= last[1] + 1) last[1] = Math.max(last[1], r[1]);
      else merged.push([r[0], r[1]]);
    }
    ranges = merged;
    for (const r of ranges) {
      let [a, b] = r;
      if (fm && a <= fm.to) {
        a = 0;
        b = Math.max(b, fm.to);
      }
      decos.between(a, b, (from, to) => {
        a = Math.min(a, from);
        b = Math.max(b, to);
      });
      tablesIn(tree, a, b, (from, to) => {
        a = Math.min(a, from);
        b = Math.max(b, to);
      });
      a = doc.lineAt(a).from;
      b = doc.lineAt(b).to;
      if (a !== r[0] || b !== r[1]) {
        r[0] = a;
        r[1] = b;
        grown = true;
      }
    }
  }
  const final = end >= doc.length;
  const ready = final ? ranges : ranges.filter((r) => r[1] < end);
  const left = final ? [] : ranges.filter((r) => r[1] >= end);
  if (!ready.length) return { decos, left };
  const add: Range<Decoration>[] = [];
  for (const [a, b] of ready) blocksIn(state, tree, a, b, hooks, add);
  return {
    decos: decos.update({
      filter: (from, to) => !ready.some(([a, b]) => from <= b && to >= a),
      filterFrom: ready[0][0],
      filterTo: ready[ready.length - 1][1],
      add,
      sort: true,
    }),
    left,
  };
}

function propertiesHtml(yaml: string): string {
  const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
  const rows: [string, string[]][] = [];
  // Set when a line is not a simple `key: value` or list item (block text,
  // nested maps, comments, not YAML at all): then the box shows the text as
  // it is, so nothing in the file is hidden.
  let raw = false;
  for (const line of yaml.split(/\r?\n/)) {
    const kv = /^([^\s:#][^:]*):\s*(.*)$/.exec(line);
    const item = /^\s*-\s+(.*)$/.exec(line);
    if (kv && !/^[|>]/.test(kv[2].trim())) {
      let v = kv[2].trim();
      let vals: string[] = [];
      if (v.startsWith("[") && v.endsWith("]")) vals = v.slice(1, -1).split(",").map((x) => x.trim()).filter(Boolean);
      else if (v) vals = [v];
      vals = vals.map((x) => x.replace(/^["']|["']$/g, ""));
      rows.push([kv[1].trim(), vals]);
    } else if (item && rows.length) {
      rows[rows.length - 1][1].push(item[1].trim().replace(/^["']|["']$/g, ""));
    } else if (line.trim()) {
      raw = true;
    }
  }
  if (raw) return `<div class="props"><pre class="props-raw">${esc(yaml)}</pre></div>`;
  if (!rows.length) return `<div class="props-empty">Properties</div>`;
  const body = rows
    .map(
      ([k, vals]) =>
        `<div class="prop"><span class="prop-key">${esc(k)}</span><span class="prop-val">${vals
          .map((v) => (/^tags?$/i.test(k) ? `<span class="tag">#${esc(v.replace(/^#/, ""))}</span>` : `<span>${esc(v)}</span>`))
          .join(" ")}</span></div>`,
    )
    .join("");
  return `<div class="props">${body}</div>`;
}

function blockField(hooks: LivePreviewHooks) {
  return StateField.define<Blocks>({
    create: (state) => blocksFor(state, hooks),
    update: (value, tr) => updateBlocks(value, tr, hooks),
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });
}

// ---------- interaction ----------

function clicks(hooks: LivePreviewHooks): Extension {
  return EditorView.domEventHandlers({
    mousedown(e, view) {
      const el = e.target as HTMLElement;
      if (e.button !== 0) return false;
      const box = el.closest(".cm-lp-task") as HTMLInputElement | null;
      if (box) {
        const pos = view.posAtDOM(box);
        const marker = view.state.sliceDoc(pos, pos + 3);
        if (/^\[[ xX]\]$/.test(marker)) {
          view.dispatch({ changes: { from: pos + 1, to: pos + 2, insert: marker[1] === " " ? "x" : " " } });
        }
        e.preventDefault();
        return true;
      }
      // Shift+click extends the selection, also over a link.
      if (e.shiftKey) return false;
      const link = el.closest(".cm-lp-link, .md-render a") as HTMLElement | null;
      if (!link) return false;
      const newTab = isMac ? e.metaKey : e.ctrlKey;
      e.preventDefault();
      if (link.dataset.wiki !== undefined) {
        hooks.openLink(link.dataset.wiki, link.dataset.sub || null, newTab);
      } else if (link.dataset.url) {
        const url = link.dataset.url;
        if (/^[a-z][a-z0-9+.-]*:/i.test(url)) hooks.openUrl(url);
        else {
          const [p, sub] = url.split("#");
          hooks.openLink(decodePath(p), sub ?? null, newTab, "markdown");
        }
      } else if (link.dataset.href !== undefined) {
        const href = link.dataset.href;
        const i = href.indexOf("#");
        hooks.openLink(i >= 0 ? href.slice(0, i) : href, i >= 0 ? href.slice(i + 1) : null, newTab);
      } else if (link.dataset.tag) {
        return false;
      } else {
        // A rendered Markdown link (table, embed): a URL, or a relative path
        // to a note or file, handled as in the reading view.
        const href = link.getAttribute("href") ?? "";
        if (/^[a-z][a-z0-9+.-]*:/i.test(href)) hooks.openUrl(href);
        else if (href && !href.startsWith("#")) {
          const [p, sub] = href.split("#");
          hooks.openLink(decodePath(p), sub ? decodePath(sub) : null, newTab);
        }
      }
      return true;
    },
  });
}

/** The word (or run of spaces or punctuation) at `pos`, as a double-click selects it. */
function groupAt(state: EditorState, pos: number, assoc: number): SelectionRange {
  const cat = state.charCategorizer(pos);
  const line = state.doc.lineAt(pos);
  const t = line.text;
  let at = pos - line.from;
  if (!t) return EditorSelection.cursor(pos);
  if (at === t.length || (assoc < 0 && at > 0)) at = findClusterBreak(t, at, false);
  const kind = cat(t.slice(at, findClusterBreak(t, at)));
  let from = at;
  let to = at;
  while (from > 0) {
    const prev = findClusterBreak(t, from, false);
    if (cat(t.slice(prev, from)) !== kind) break;
    from = prev;
  }
  while (to < t.length) {
    const next = findClusterBreak(t, to);
    if (cat(t.slice(to, next)) !== kind) break;
    to = next;
  }
  return EditorSelection.range(line.from + from, line.from + to);
}

/** Where the last single click landed, in the text as it looked then. */
let lastClick: { view: EditorView; doc: Text; time: number; x: number; y: number; pos: number; assoc: number } | null = null;

/**
 * The first click of a double-click moves the cursor to the line, which shows
 * the line's hidden syntax and shifts its text. The second click then lands
 * on other text than the user aimed at. So a double-click at the same spot
 * selects the word under the first click (dragging extends it by words).
 */
const doubleClick = EditorView.mouseSelectionStyle.of((view, e) => {
  if (e.button !== 0) return null;
  const { clientX: x, clientY: y } = e;
  if (e.detail === 1) {
    lastClick = { view, doc: view.state.doc, time: e.timeStamp, x, y, ...view.posAndSideAtCoords({ x, y }, false) };
    return null;
  }
  const first = lastClick;
  if (e.detail !== 2 || e.shiftKey || e.altKey || e.metaKey || e.ctrlKey) return null;
  if (!first || first.view !== view || first.doc !== view.state.doc || e.timeStamp - first.time > 1000) return null;
  if (Math.abs(first.x - x) > 4 || Math.abs(first.y - y) > 4) return null;
  let start = groupAt(view.state, first.pos, first.assoc);
  return {
    update(u) {
      if (u.docChanged) start = start.map(u.changes);
    },
    get(cur) {
      if (cur.clientX === x && cur.clientY === y) return EditorSelection.create([start]);
      const at = view.posAndSideAtCoords({ x: cur.clientX, y: cur.clientY }, false);
      const r = groupAt(view.state, at.pos, at.assoc);
      const from = Math.min(start.from, r.from);
      const to = Math.max(start.to, r.to);
      return EditorSelection.create([r.from < start.from ? EditorSelection.range(to, from) : EditorSelection.range(from, to)]);
    },
  };
});

function decodePath(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/**
 * Links rendered in widgets are real `<a href>` elements, opened on mousedown
 * above. The click that follows must never be followed by the web view
 * itself: that would replace the app with the target page. A plain listener,
 * so links in widgets that ignore events are covered too.
 */
const noFollow = ViewPlugin.define((view) => {
  const cancel = (e: MouseEvent) => {
    if (e.target instanceof Element && e.target.closest("a[href]")) e.preventDefault();
  };
  view.dom.addEventListener("click", cancel);
  view.dom.addEventListener("auxclick", cancel);
  return {
    destroy() {
      view.dom.removeEventListener("click", cancel);
      view.dom.removeEventListener("auxclick", cancel);
    },
  };
});

/** The Live Preview extension (put inside `livePreviewCompartment`). */
export function livePreview(hooks: LivePreviewHooks): Extension {
  return [
    inlinePlugin(hooks),
    embedVersion,
    blockField(hooks),
    clicks(hooks),
    doubleClick,
    noFollow,
    EditorView.editorAttributes.of({ class: "cm-live-preview" }),
  ];
}

// Exposed for tests.
export const _internal = { buildBlocks, blockField, embedVersion, propertiesHtml, RangeSet };
