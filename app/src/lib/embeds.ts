// Fill `<span class="embed">` placeholders produced by the Markdown renderer
// with the rendered content of the embedded note (or a section of it).

import { backend } from "./backend";
import type { LinkIndex } from "./links";
import { headingsOf, renderMarkdown } from "./markdown";
import { displayName, extension, isMarkdown } from "./paths";

const MAX_DEPTH = 3;

function escapeHtml(s: string): string {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
}

/** The embed's heading: the file name, a link that opens it. */
function titleLink(path: string, sub: string | null): string {
  const title = displayName(path) + (sub ? ` › ${sub}` : "");
  return `<a class="embed-title internal-link" data-href="${escapeHtml(path)}${sub ? "#" + escapeHtml(sub) : ""}" href="#">${escapeHtml(title)}</a>`;
}

/**
 * The part of `text` under heading `sub` (up to the next heading of the same
 * or higher level), or the block ending in `^id` when `sub` starts with `^`.
 * Returns null if not found.
 */
export function extractSection(text: string, sub: string): string | null {
  const lines = text.split("\n");
  if (sub.startsWith("^")) {
    const id = sub.slice(1);
    const re = new RegExp(`\\s\\^${id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*$`);
    const i = lines.findIndex((l) => re.test(l));
    if (i < 0) return null;
    let start = i;
    while (start > 0 && lines[start - 1].trim() !== "") start--;
    return lines.slice(start, i + 1).join("\n").replace(re, "");
  }
  const want = sub.trim().toLowerCase();
  const heads = headingsOf(text);
  const i = heads.findIndex((h) => h.text.toLowerCase() === want);
  if (i < 0) return null;
  const next = heads.slice(i + 1).find((h) => h.level <= heads[i].level);
  return lines.slice(heads[i].line, next?.line ?? lines.length).join("\n").trimEnd();
}

export async function fillEmbeds(
  root: HTMLElement,
  sourcePath: string,
  links: LinkIndex | null,
  depth = 0,
  seen: Set<string> = new Set(),
): Promise<void> {
  const els = [...root.querySelectorAll<HTMLElement>("span.embed[data-target]:not([data-filled])")];
  await Promise.all(
    els.map(async (el) => {
      el.dataset.filled = "1";
      const target = el.dataset.target ?? "";
      const sub = el.dataset.subpath || null;
      try {
        const path = await backend.resolveLink(target, sourcePath);
        if (!path) {
          el.innerHTML = `<span class="embed-missing">"${escapeHtml(target)}" does not exist yet.</span>`;
          return;
        }
        const key = `${path}#${sub ?? ""}`;
        if (depth >= MAX_DEPTH || seen.has(key)) {
          el.innerHTML = `<span class="embed-missing">Embed of ${escapeHtml(displayName(path))} skipped (nested too deep).</span>`;
          return;
        }
        const text = isMarkdown(path) ? (await backend.readNote(path)).content : await backend.readTextFile(path);
        if (text == null) {
          // A PDF, an archive or a big file: a card with its name, never its bytes.
          const kind = extension(path) ? `${extension(path).toUpperCase()} file` : "File";
          el.innerHTML = `${titleLink(path, null)}<div class="embed-body"><span class="embed-file">${escapeHtml(kind)}, opens in another app.</span></div>`;
          return;
        }
        const part = sub ? extractSection(text, sub) : text;
        if (part == null) {
          el.innerHTML = `<span class="embed-missing">"${escapeHtml(sub ?? "")}" not found in ${escapeHtml(displayName(path))}.</span>`;
          return;
        }
        const body = isMarkdown(path)
          ? await renderMarkdown(part, { links, sourcePath: path })
          : `<pre>${escapeHtml(part)}</pre>`;
        el.innerHTML = `${titleLink(path, sub)}<div class="embed-body">${body}</div>`;
        await fillEmbeds(el, path, links, depth + 1, new Set([...seen, key]));
      } catch (e) {
        el.innerHTML = `<span class="embed-missing">Could not embed "${escapeHtml(target)}": ${escapeHtml(String((e as { detail?: string })?.detail ?? e))}</span>`;
      }
    }),
  );
}
