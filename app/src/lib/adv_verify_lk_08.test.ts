// Reproduction for FINDING-085 (UI side).
//
// The UI finds where frontmatter ends in markdown.ts stripFrontmatter (reading
// view), editor/livePreview.ts buildBlocks (Live Preview properties box) and
// app.svelte.ts loadTab (initial cursor); the core does it in
// crates/cairn-core/src/parse.rs split_frontmatter. A regex such as
//   /^---\r?\n([\s\S]*?)\r?\n(?:---|\.\.\.)[ \t]*(?:\r?\n|$)/
// never matches an empty block, and a core that wants a closing line that is
// exactly "---"/"..." disagrees with it. All of them use one rule (markdown.ts
// FRONTMATTER_RE): trailing spaces or tabs after the closing fence, and an
// empty block, are allowed.
// The core numbers below come from crates/cairn-core/tests/adv_verify_lk_08.rs.
//
// Run: cd app && npx vitest run src/lib/adv_verify_lk_08.test.ts
import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { yamlFrontmatter } from "@codemirror/lang-yaml";
import { renderUnsafe, stripFrontmatter } from "./markdown";
import { _internal } from "./editor/livePreview";

const hooks = {
  linkIndex: () => null,
  notePath: () => "Note.md",
  openLink: () => {},
  openUrl: () => {},
  openImage: () => {},
};

/** Live Preview: does the properties box replace the top of the note, and up to which offset? */
function lpPropsBox(doc: string): { to: number; html: string } | null {
  // Cursor on the last line, away from the frontmatter (as after loadTab).
  const state = EditorState.create({
    doc,
    selection: { anchor: doc.length },
    extensions: [yamlFrontmatter({ content: markdown({ base: markdownLanguage }) })],
  });
  const set = _internal.buildBlocks(state, hooks);
  let found: { to: number; html: string } | null = null;
  set.between(0, doc.length, (from, to, deco) => {
    const w = (deco.spec as { widget?: { className?: string; html?: string } }).widget;
    if (from === 0 && w?.className === "cm-lp-props") found = { to, html: w.html ?? "" };
  });
  return found;
}

const TRAILING = "---\ntitle: Hello\ntags: [alpha]\nrelated: \"[[Other]]\"\n---  \n# Real\n";
const TRAILING_RULE = "---\ntitle: Hello\n---  \n# Intro\nSee [[Linked]] #intro\n\n---\n\nRest\n";
const EMPTY_INTRO = "---\n---\nIntro paragraph with [[Intro]] #introtag\n\n---\n\nRest\n";

describe("FINDING-085: closing fence with trailing spaces", () => {
  it("UI treats it as frontmatter: reading view hides the YAML, Live Preview shows a properties box", () => {
    const s = stripFrontmatter(TRAILING);
    expect(s.frontmatter).toBe('title: Hello\ntags: [alpha]\nrelated: "[[Other]]"');
    expect(renderUnsafe(TRAILING)).not.toContain("title: Hello");
    const box = lpPropsBox(TRAILING);
    expect(box).not.toBeNull();
    expect(box!.html).toContain('<span class="tag">#alpha</span>');
    // A core with the strict rule sees no frontmatter here: tags=[], outline
    // ["title: Hellotags: [alpha]related: \"[[Other]]\"", "Real"], links ["Other"].
  });

  it("with a later '---' rule the UI closes at the '---  ' line and shows the first section", () => {
    expect(stripFrontmatter(TRAILING_RULE).frontmatter).toBe("title: Hello");
    const html = renderUnsafe(TRAILING_RULE);
    expect(html).toContain("Intro</h1>");
    expect(html).toContain("Linked");
    // A core with the strict rule closes at the later rule: body_start=58, links=[], tags=[], headings=[].
  });

  it("FINDING-085: UI and core agree on the body start for '---  '", () => {
    // Core body_start for TRAILING is 58 (0 would mean no frontmatter).
    const s = stripFrontmatter(TRAILING);
    expect(s.frontmatter === null ? 0 : TRAILING.length - s.body.length).toBe(58);
  });
});

describe("FINDING-085: empty '---\\n---' frontmatter", () => {
  it("reading view hides the empty block, like the core", () => {
    const src = "---\n---\nbody";
    expect(stripFrontmatter(src)).toEqual({ body: "body", frontmatter: "" });
    expect(renderUnsafe(src)).toBe("<p>body</p>\n");
  });

  it("with a later '---' rule, the empty block still ends at its second line, before the intro", () => {
    const s = stripFrontmatter(EMPTY_INTRO);
    expect(s.frontmatter).toBe("");
    expect(s.body.startsWith("Intro paragraph")).toBe(true);
    const box = lpPropsBox(EMPTY_INTRO);
    expect(box).not.toBeNull();
    // Live Preview shows an empty "Properties" box over the two dash lines only.
    expect(box!.html).toBe('<div class="props-empty">Properties</div>');
    expect(box!.to).toBe(7);
  });

  it("FINDING-085: the intro paragraph after an empty frontmatter block is visible in the reading view", () => {
    // Core: frontmatter={}, body_start=8, indexes [[Intro]] and #introtag.
    expect(renderUnsafe(EMPTY_INTRO)).toContain("Intro paragraph");
  });

  it("FINDING-085: the intro paragraph is not hidden by the Live Preview properties box", () => {
    const box = lpPropsBox(EMPTY_INTRO);
    expect(box === null || box.to <= 7).toBe(true);
  });
});

describe("FINDING-052: frontmatter after a BOM", () => {
  it("the reading view hides it and Live Preview shows the properties box", () => {
    const src = "\uFEFF---\ntags: [alpha]\n---\n# Head\n";
    expect(renderUnsafe(src)).toBe("<h1>Head</h1>\n");
    const box = lpPropsBox(src);
    expect(box?.to).toBe(src.indexOf("\n# Head"));
    expect(box!.html).toContain('<span class="tag">#alpha</span>');
  });
});
