// Markdown to HTML for the preview pane: markdown-it with wikilinks, tags
// and task lists, sanitized with DOMPurify.

import MarkdownIt from "markdown-it";
import DOMPurify from "dompurify";

type MD = InstanceType<typeof MarkdownIt>;
type StateInline = Parameters<Parameters<MD["inline"]["ruler"]["push"]>[1]>[0];
type CoreState = Parameters<Parameters<MD["core"]["ruler"]["push"]>[1]>[0];
import { splitWikilink, type LinkIndex } from "./links";
import { vaultUrl } from "./backend";
import { AUDIO_EXTS, IMAGE_EXTS, VIDEO_EXTS, extension, parent, resolveRelative } from "./paths";

export interface RenderContext {
  links: LinkIndex | null;
  /** Path of the note being rendered, for relative links and images. */
  sourcePath?: string;
}

/** `![[pic.png|300]]` or `|300x200` sets the size. */
function sizeAttrs(alias: string | null): string {
  const m = alias ? /^(\d+)(?:x(\d+))?$/.exec(alias.trim()) : null;
  if (!m) return "";
  return ` width="${m[1]}"${m[2] ? ` height="${m[2]}"` : ""}`;
}

function escapeAttr(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function wikilinkPlugin(md: MD) {
  md.inline.ruler.before("link", "wikilink", (state: StateInline, silent: boolean) => {
    const src = state.src;
    let pos = state.pos;
    const embed = src.charCodeAt(pos) === 0x21; /* ! */
    if (embed) pos++;
    if (src.charCodeAt(pos) !== 0x5b || src.charCodeAt(pos + 1) !== 0x5b) return false;
    const end = src.indexOf("]]", pos + 2);
    if (end < 0) return false;
    const inner = src.slice(pos + 2, end);
    if (!inner || /[[\]\n]/.test(inner)) return false;
    if (!silent) {
      const parts = splitWikilink(inner);
      const tok = state.push("wikilink", "", 0);
      tok.meta = { ...parts, embed, raw: inner };
    }
    state.pos = end + 2;
    return true;
  });
  md.renderer.rules.wikilink = (tokens, idx, _opts, envAny) => {
    const env = envAny as RenderContext | undefined;
    const { target, subpath, alias, embed, raw } = tokens[idx].meta as {
      target: string;
      subpath: string | null;
      alias: string | null;
      embed: boolean;
      raw: string;
    };
    const exists = !target || (env?.links?.exists(target, env.sourcePath ?? "") ?? true);
    if (embed && target) {
      const resolved = env?.links?.resolve(target, env.sourcePath ?? "") ?? null;
      const ext = extension(resolved ?? target);
      if (resolved && IMAGE_EXTS.has(ext)) {
        const alt = alias && !/^\d+(x\d+)?$/.test(alias) ? alias : target;
        return `<img class="embed-image" src="${escapeAttr(vaultUrl(resolved))}" data-path="${escapeAttr(resolved)}" alt="${escapeAttr(alt)}"${sizeAttrs(alias)}>`;
      }
      if (resolved && AUDIO_EXTS.has(ext)) return `<audio class="embed-media" controls src="${escapeAttr(vaultUrl(resolved))}"></audio>`;
      if (resolved && VIDEO_EXTS.has(ext)) return `<video class="embed-media" controls src="${escapeAttr(vaultUrl(resolved))}"></video>`;
      // Notes (and missing targets) are filled in after rendering, see embeds.ts.
      return `<span class="embed" data-target="${escapeAttr(target)}" data-subpath="${escapeAttr(subpath ?? "")}"></span>`;
    }
    const label = alias ?? (target ? target + (subpath ? ` › ${subpath}` : "") : subpath ?? raw);
    const href = target + (subpath ? `#${subpath}` : "");
    const cls = `internal-link${exists ? "" : " is-unresolved"}${embed ? " is-embed" : ""}`;
    return `<a class="${cls}" data-href="${escapeAttr(href)}" href="#">${md.utils.escapeHtml(label)}</a>`;
  };
  // Relative image paths point into the vault.
  const defaultImage = md.renderer.rules.image!;
  md.renderer.rules.image = (tokens, idx, opts, envAny, self) => {
    const env = envAny as RenderContext | undefined;
    const tok = tokens[idx];
    const src = String(tok.attrGet("src") ?? "");
    // As in the core, the part after '#' is not part of the path.
    let rel = src.split("#")[0];
    if (rel && !/^[a-z][a-z0-9+.-]*:/i.test(src) && !src.startsWith("//")) {
      try {
        rel = decodeURIComponent(rel);
      } catch {}
      const direct = resolveRelative(parent(env?.sourcePath ?? ""), rel);
      const viaName = env?.links?.resolve(rel, env.sourcePath ?? "") ?? null;
      // The relative path only if that exact file exists (the vault://
      // handler reads exact paths), else what the core would resolve.
      const known = direct && env?.links?.has(direct) ? direct : viaName;
      const path = known ?? direct;
      if (path) tok.attrSet("src", vaultUrl(path));
      // A click in the reading view opens the image in an image tab.
      if (known && IMAGE_EXTS.has(extension(known))) tok.attrSet("data-path", known);
    }
    return defaultImage(tokens, idx, opts, envAny, self);
  };
}

function tagPlugin(md: MD) {
  md.inline.ruler.push("tag", (state: StateInline, silent: boolean) => {
    const src = state.src;
    const pos = state.pos;
    if (src.charCodeAt(pos) !== 0x23 /* # */) return false;
    // After whitespace, ',' or ';', like TAG_RE in the core's parse.rs.
    const prev = pos > 0 ? src[pos - 1] : " ";
    if (!/[\s,;]/.test(prev)) return false;
    // \p{M}: combining marks (NFD accents, Devanagari or Thai vowel signs)
    const m = /^#([\p{L}\p{M}\p{N}_/-]*[\p{L}_/-][\p{L}\p{M}\p{N}_/-]*)/u.exec(src.slice(pos));
    if (!m) return false;
    if (!silent) {
      const tok = state.push("tag", "", 0);
      tok.content = m[1];
    }
    state.pos += m[0].length;
    return true;
  });
  md.renderer.rules.tag = (tokens, idx) => {
    const t = md.utils.escapeHtml(tokens[idx].content);
    return `<a class="tag" data-tag="${t}" href="#">#${t}</a>`;
  };
}

function taskListPlugin(md: MD) {
  md.core.ruler.after("inline", "tasklist", (state: CoreState) => {
    const toks = state.tokens;
    for (let i = 2; i < toks.length; i++) {
      const t = toks[i];
      if (t.type !== "inline" || toks[i - 1].type !== "paragraph_open" || toks[i - 2].type !== "list_item_open") continue;
      const m = /^\[([ xX])\]\s/.exec(t.content);
      if (!m || !t.children?.length) continue;
      const first = t.children[0];
      if (first.type !== "text") continue;
      first.content = first.content.replace(/^\[[ xX]\]\s/, "");
      const box = new state.Token("html_inline", "", 0);
      box.content = `<input type="checkbox" class="task" disabled${m[1] !== " " ? " checked" : ""}> `;
      t.children.unshift(box);
      toks[i - 2].attrJoin("class", "task-item");
    }
  });
}

const md = new MarkdownIt({ html: true, linkify: true, typographer: false, breaks: false });
md.use(wikilinkPlugin).use(tagPlugin).use(taskListPlugin);

/**
 * YAML frontmatter at the top of a note: an optional BOM, a `---` line, then
 * up to the first `---` or `...` line (trailing spaces or tabs allowed; the
 * block may be empty). Group 1 is the YAML, if any. Keep in step with
 * split_frontmatter in crates/cairn-core/src/parse.rs.
 */
export const FRONTMATTER_RE = /^\uFEFF?---\r?\n(?:([\s\S]*?)\r?\n)??(?:---|\.\.\.)[ \t]*(?:\r?\n|$)/;

/** Strip YAML frontmatter (and a leading BOM); the frontmatter is shown separately. */
export function stripFrontmatter(src: string): { body: string; frontmatter: string | null } {
  const m = FRONTMATTER_RE.exec(src);
  if (!m) return { body: src.replace(/^\uFEFF/, ""), frontmatter: null };
  return { body: src.slice(m[0].length), frontmatter: m[1] ?? "" };
}

// Without the wikilink and tag plugins: in heading text they are plain text,
// as in the core.
const plainMd = new MarkdownIt({ html: true });

type InlineToken = { type: string; content: string; children: InlineToken[] | null };

function inlineText(toks: InlineToken[]): string {
  let s = "";
  for (const t of toks) {
    if (t.type === "text" || t.type === "code_inline") s += t.content;
    else if (t.type === "softbreak" || t.type === "hardbreak") s += " ";
    else if (t.type === "image") s += inlineText(t.children ?? []);
  }
  return s;
}

/**
 * The headings of a note as the core's outline lists them (parse.rs): ATX
 * and setext, also in lists and quotes, not in frontmatter or code; the text
 * without formatting, line breaks as spaces. `line` is 0-based in `src`.
 */
export function headingsOf(src: string): { level: number; text: string; line: number }[] {
  const { body } = stripFrontmatter(src);
  const offset = src.slice(0, src.length - body.length).split("\n").length - 1;
  const toks = plainMd.parse(body, {});
  const out: { level: number; text: string; line: number }[] = [];
  toks.forEach((t, i) => {
    if (t.type === "heading_open" && t.map)
      out.push({ level: Number(t.tag.slice(1)), text: inlineText(toks[i + 1].children ?? []).trim(), line: t.map[0] + offset });
  });
  return out;
}

const PURIFY_OPTS = {
  ADD_ATTR: ["data-href", "data-tag", "data-target", "data-subpath", "data-path", "target"],
  ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto|tel|vault):|[^a-z]|[a-z+.-]+(?:[^a-z+.\-:]|$))/i,
  FORBID_TAGS: ["style", "script", "iframe", "form"],
};

/** Render without sanitizing (used in tests). */
export function renderUnsafe(src: string, ctx: RenderContext = { links: null }): string {
  return md.render(stripFrontmatter(src).body, { ...ctx });
}

/** Render and sanitize. Needs a DOM (DOMPurify). */
export function renderMarkdownSync(src: string, ctx: RenderContext): string {
  return DOMPurify.sanitize(renderUnsafe(src, ctx), PURIFY_OPTS) as unknown as string;
}

export async function renderMarkdown(src: string, ctx: RenderContext): Promise<string> {
  return renderMarkdownSync(src, ctx);
}
