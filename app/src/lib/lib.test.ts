import { describe, expect, it, vi } from "vitest";
import { buildTree, flatten } from "./tree";
import { LinkIndex, openWikilinkQuery, splitWikilink, wikilinkAt } from "./links";
import { renderUnsafe, stripFrontmatter } from "./markdown";
import { _internal as lp } from "./editor/livePreview";
import { displayName, isInside, rebase, stem } from "./paths";
import { back, closeOnBack } from "./back";
import type { FileStat, WriteResult } from "./types";

const f = (path: string, kind: "file" | "dir" = "file"): FileStat => ({ path, kind, size: 0, mtime: 0 });

describe("paths", () => {
  it("handles names", () => {
    expect(stem("a/b.c.md")).toBe("b.c");
    expect(displayName("a/Note.md")).toBe("Note");
    expect(displayName("a/pic.png")).toBe("pic.png");
    expect(isInside("a/b", "a")).toBe(true);
    expect(isInside("ab", "a")).toBe(false);
    expect(rebase("a/b/c.md", "a/b", "x")).toBe("x/c.md");
  });
});

describe("Android Back", () => {
  it("closes the overlay opened last, and says when none is open", () => {
    const closed: string[] = [];
    const offDrawer = closeOnBack(() => closed.push("drawer"));
    const offMenu = closeOnBack(() => closed.push("menu"));
    const offDialog = closeOnBack(() => closed.push("dialog"));
    offMenu(); // the menu closed by itself
    expect(back()).toBe(true);
    offDialog(); // its teardown runs after Back closed it
    expect(back()).toBe(true);
    offDrawer();
    expect(back()).toBe(false);
    expect(closed).toEqual(["dialog", "drawer"]);
  });
});

describe("tree", () => {
  it("builds a sorted tree, folders first, natural order", () => {
    const t = buildTree([f("b.md"), f("a", "dir"), f("a/z.md"), f("a/note 10.md"), f("a/note 2.md"), f("A.md")]);
    expect(t.children.map((c) => c.name)).toEqual(["a", "A.md", "b.md"]);
    expect(t.children[0].children.map((c) => c.name)).toEqual(["note 2.md", "note 10.md", "z.md"]);
  });

  it("flattens only expanded folders", () => {
    const t = buildTree([f("d", "dir"), f("d/x.md"), f("e", "dir"), f("e/y.md")]);
    const rows = flatten(t, new Set(["e"]));
    expect(rows.map((r) => `${r.depth}:${r.node.path}`)).toEqual(["0:d", "0:e", "1:e/y.md"]);
  });

  it("gives each row its place among its siblings", () => {
    const t = buildTree([f("d", "dir"), f("d/x.md"), f("d/z.md"), f("a.md")]);
    const rows = flatten(t, new Set(["d"]));
    expect(rows.map((r) => `${r.node.path} ${r.pos}/${r.size}`)).toEqual(["d 1/2", "d/x.md 1/2", "d/z.md 2/2", "a.md 2/2"]);
  });

  it("synthesizes missing parents", () => {
    const t = buildTree([f("x/y/z.md")]);
    expect(t.children[0].path).toBe("x");
    expect(t.children[0].children[0].children[0].path).toBe("x/y/z.md");
  });
});

describe("links", () => {
  const idx = new LinkIndex([f("Note.md"), f("a/Dup.md"), f("b/Dup.md"), f("img/pic.png"), f("dir", "dir")]);

  it("picks the shortest unambiguous link text", () => {
    expect(idx.linkText("Note.md")).toBe("Note");
    expect(idx.linkText("a/Dup.md")).toBe("a/Dup");
    expect(idx.linkText("img/pic.png")).toBe("pic.png");
  });

  it("knows which targets exist", () => {
    expect(idx.exists("note")).toBe(true);
    expect(idx.exists("Note.md")).toBe(true);
    expect(idx.exists("b/Dup")).toBe(true);
    expect(idx.exists("c/Dup")).toBe(false);
    expect(idx.exists("Missing")).toBe(false);
    expect(idx.exists("pic.png")).toBe(true);
  });

  it("splits wikilinks", () => {
    expect(splitWikilink("A#H|alias")).toEqual({ target: "A", subpath: "H", alias: "alias" });
    expect(splitWikilink("A\\|x")).toEqual({ target: "A", subpath: null, alias: "x" });
    expect(splitWikilink("#Only")).toEqual({ target: "", subpath: "Only", alias: null });
  });

  it("finds the link under the cursor", () => {
    const line = "see [[One]] and ![[Two|t]]";
    expect(wikilinkAt(line, 6)?.target).toBe("One");
    expect(wikilinkAt(line, 20)).toMatchObject({ target: "Two", embed: true, alias: "t" });
    expect(wikilinkAt(line, 13)).toBeNull();
  });

  it("detects an open [[ query", () => {
    expect(openWikilinkQuery("text [[No")).toEqual({ query: "No", start: 7 });
    expect(openWikilinkQuery("text [[No]] x")).toBeNull();
    expect(openWikilinkQuery("[[a|b")).toBeNull();
  });
});

describe("markdown", () => {
  const idx = new LinkIndex([f("Exists.md")]);

  it("renders wikilinks with resolution classes", () => {
    const html = renderUnsafe("[[Exists]] [[Missing|label]] [[Exists#H]]", { links: idx });
    expect(html).toContain('<a class="internal-link" data-href="Exists" href="#">Exists</a>');
    expect(html).toContain('class="internal-link is-unresolved" data-href="Missing" href="#">label</a>');
    expect(html).toContain('class="internal-link" data-href="Exists#H" href="#">Exists › H</a>');
  });

  it("does not render links inside code", () => {
    const html = renderUnsafe("`[[Nope]]`\n\n```\n[[Nope]]\n```", { links: idx });
    expect(html).not.toContain("internal-link");
  });

  it("renders tags and task lists, strips frontmatter", () => {
    const html = renderUnsafe("---\ntitle: x\n---\n- [ ] todo #tag\n- [x] done\n\nC# is not a tag", { links: idx });
    expect(html).toContain('<a class="tag" data-tag="tag" href="#">#tag</a>');
    expect(html).toContain('<input type="checkbox" class="task" disabled> todo');
    expect(html).toContain("checked");
    expect(html).not.toContain("title: x");
    expect(html).toContain("C# is not a tag");
  });

  it("splits frontmatter", () => {
    expect(stripFrontmatter("---\na: 1\n---\nbody")).toEqual({ body: "body", frontmatter: "a: 1" });
    expect(stripFrontmatter("no fm").frontmatter).toBeNull();
    // Same rule as the core: a BOM is not text, the closing line may end in
    // spaces or tabs, and the block may be empty.
    expect(stripFrontmatter("\uFEFF---\na: 1\n---\nbody")).toEqual({ body: "body", frontmatter: "a: 1" });
    expect(stripFrontmatter("\uFEFF# T")).toEqual({ body: "# T", frontmatter: null });
    expect(renderUnsafe("\uFEFF# T")).toBe("<h1>T</h1>\n");
    expect(stripFrontmatter("---\r\na: 1\r\n--- \t\r\nbody")).toEqual({ body: "body", frontmatter: "a: 1" });
    expect(stripFrontmatter("---\n---\nbody")).toEqual({ body: "body", frontmatter: "" });
    expect(stripFrontmatter("--- \na: 1\n---\nbody").frontmatter).toBeNull();
  });
});

import { extractSection } from "./embeds";

describe("embeds and images", () => {
  const idx = new LinkIndex([f("img/pic.png"), f("Note.md"), f("a/b/local.png")]);

  it("renders image embeds as vault URLs with sizes", () => {
    const html = renderUnsafe("![[pic.png|200]] ![[Note]] ![[Note#Sec]]", { links: idx, sourcePath: "x.md" });
    // data-path: a click in the reading view opens the image in an image tab.
    expect(html).toContain('<img class="embed-image" src="vault://localhost/img/pic.png" data-path="img/pic.png" alt="pic.png" width="200">');
    expect(html).toContain('<span class="embed" data-target="Note" data-subpath=""></span>');
    expect(html).toContain('data-subpath="Sec"');
  });

  it("rewrites relative markdown images", () => {
    const html = renderUnsafe("![a](local.png) ![b](https://e.com/x.png)", { links: idx, sourcePath: "a/b/n.md" });
    expect(html).toContain('src="vault://localhost/a/b/local.png"');
    expect(html).toContain('src="https://e.com/x.png"');
    // Only the vault's own image can be opened in an image tab.
    expect(html).toContain('data-path="a/b/local.png"');
    expect(html.match(/data-path=/g)).toHaveLength(1);
  });

  it("extracts heading sections and blocks", () => {
    const text = "# A\nintro\n## B\nb text\n### C\nc\n## D\nd\n\npara one\npara two ^blk\n";
    expect(extractSection(text, "B")).toBe("## B\nb text\n### C\nc");
    expect(extractSection(text, "d")).toBe("## D\nd\n\npara one\npara two ^blk");
    expect(extractSection(text, "^blk")).toBe("para one\npara two");
    expect(extractSection(text, "Missing")).toBeNull();
  });

  it("resolves like the core index", () => {
    const i2 = new LinkIndex([f("q/Deep.md"), f("x/Deep.md"), f("x/y/Deep.md")]);
    expect(i2.resolve("Deep", "x/y/other.md")).toBe("x/y/Deep.md");
    expect(i2.resolve("Deep", "z.md")).toBe("q/Deep.md");
    expect(i2.resolve("y/Deep", "z.md")).toBe("x/y/Deep.md");
    expect(i2.resolve("nope")).toBeNull();
  });
});

import { parseManifest, permissionFor, PluginHost } from "./plugins";
import { backend } from "./backend";

describe("plugins", () => {
  it("parses the header", () => {
    const m = parseManifest("x.js", "// @name Stamp\n// @description Adds a stamp\n// @permissions editor, read bogus\ncode()");
    expect(m).toEqual({ file: "x.js", name: "Stamp", description: "Adds a stamp", permissions: ["editor", "read"] });
    expect(parseManifest("y.js", "code()").name).toBe("y");
  });

  it("maps API methods to permissions", () => {
    expect(permissionFor("notes.read")).toBe("read");
    expect(permissionFor("notes.write")).toBe("write");
    expect(permissionFor("editor.replaceSelection")).toBe("editor");
    expect(permissionFor("ui.toast")).toBeNull();
    expect(permissionFor("fs.delete")).toBeUndefined();
  });

  it("refuses calls without permission", async () => {
    const host = new PluginHost({
      notePaths: () => ["a.md"],
      activePath: () => null,
      getSelection: () => "",
      replaceSelection: () => true,
      toast: () => {},
    });
    const m = parseManifest("p.js", "// @permissions editor");
    await expect(host.handle(m, "notes.list", [])).rejects.toThrow(/does not have the "read" permission/);
    await expect(host.handle(m, "notes.write", ["a.md", "x"])).rejects.toThrow(/"write"/);
    await expect(host.handle(m, "secret.thing", [])).rejects.toThrow(/unknown API/);
    await expect(host.handle(m, "editor.getSelection", [])).resolves.toBe("");
  });

  it("reads and writes notes only inside the vault", async () => {
    const read = vi.spyOn(backend, "readNote").mockResolvedValue({ content: "c", hash: "h" });
    const write = vi.spyOn(backend, "writeNote").mockResolvedValue({} as WriteResult);
    const create = vi.spyOn(backend, "createNote").mockResolvedValue({} as WriteResult);
    try {
      const host = new PluginHost({ notePaths: () => [], activePath: () => null, getSelection: () => "", replaceSelection: () => true, toast: () => {} });
      const m = parseManifest("p.js", "// @permissions read write");
      await expect(host.handle(m, "notes.read", ["a.md"])).resolves.toBe("c");
      await host.handle(m, "notes.write", ["a.md", "x"]);
      read.mockRejectedValueOnce({ kind: "notFound", detail: "b.md" });
      await host.handle(m, "notes.write", ["b.md", "y"]);
      // `true`: the backend refuses a path that a symlink leads out of the vault.
      expect(read.mock.calls).toEqual([["a.md", true], ["a.md", true], ["b.md", true]]);
      expect(write.mock.calls).toEqual([["a.md", "x", "h", true]]);
      expect(create.mock.calls).toEqual([["b.md", "y", true]]);
    } finally {
      vi.restoreAllMocks();
    }
  });
});

describe("Live Preview properties box", () => {
  it("shows simple keys and lists as properties", () => {
    const html = lp.propertiesHtml("title: Trip\ntags:\n- a\n  - b\naliases: [x, y]");
    expect(html).toContain('<span class="prop-key">title</span><span class="prop-val"><span>Trip</span>');
    expect(html).toContain('<span class="tag">#a</span> <span class="tag">#b</span>');
    expect(html).toContain("<span>x</span> <span>y</span>");
    expect(html).not.toContain("props-raw");
  });

  it("shows the text as it is when a line is not a simple property (FINDING-093)", () => {
    for (const yaml of [
      "title: Trip\ndescription: |\n  SECRET PLAN\n  line two",
      "location:\n  city: PARIS",
      "title: Trip\n# a comment",
      "Intro paragraph that is not YAML.\n",
    ]) {
      const html = lp.propertiesHtml(yaml);
      expect(html).toContain('<pre class="props-raw">');
      for (const word of ["SECRET PLAN", "PARIS", "# a comment", "Intro paragraph"].filter((w) => yaml.includes(w))) {
        expect(html).toContain(word);
      }
    }
    expect(lp.propertiesHtml("note: <b>&</b>\n  nested")).toContain("note: &lt;b&gt;&amp;&lt;/b&gt;");
  });
});
