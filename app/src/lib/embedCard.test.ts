// The card of an embedded file that does not show as text (a PDF, an archive,
// a big file) says what a click on its title does. On the desktop, the
// backend's open_externally decides: the card says that the file opens in
// another app, or that Cairn does not open it (a type it refuses, a link to
// one, or a text file marked as executable). Android cannot hand files to
// other apps yet (a tap on the title only shows a toast), so there the card
// says that instead.
//
// Run: cd app && npx vitest run src/lib/embedCard.test.ts

import { beforeEach, describe, expect, it, vi } from "vitest";

const platform = vi.hoisted(() => ({ mobile: false }));
const checked = vi.hoisted(() => [] as string[]);

vi.mock("./platform", () => ({
  get isAndroid() {
    return platform.mobile;
  },
  get isMobile() {
    return platform.mobile;
  },
  narrowQuery: () => null,
  vaultLabel: (root: string) => root,
}));

vi.mock("./backend", () => ({
  vaultUrl: (p: string) => `vault://localhost/${p}`,
  backend: {
    resolveLink: async (target: string) => target,
    readNote: async () => ({ content: "", hash: "h" }),
    // Text only for .txt; null, as for a binary or a big file, for the rest.
    readTextFile: async (path: string) => (path.endsWith(".txt") ? "plain text\n" : null),
    // What open_externally would do, by name for this test.
    openExternallyCheck: async (path: string) => {
      checked.push(path);
      if (path === "gone.pdf") throw new Error("not found");
      if (path === "run.log") return "executable";
      if (path === "looks.pdf") return "linktype";
      if (path === "deep.pdf") return "path";
      return path.endsWith(".pdf") || path.endsWith(".log") ? "opens" : "type";
    },
  },
}));

import { fillEmbeds } from "./embeds";

type FakeSpan = { dataset: Record<string, string>; innerHTML: string; querySelectorAll: () => FakeSpan[] };

/** Fill one embed of `target` in a stand-in for the rendered note and return its HTML. */
async function embed(target: string): Promise<string> {
  const span: FakeSpan = { dataset: { target }, innerHTML: "", querySelectorAll: () => [] };
  const root = { querySelectorAll: () => [span].filter((s) => !s.dataset.filled) };
  await fillEmbeds(root as unknown as HTMLElement, "Note.md", null);
  return span.innerHTML;
}

const cardText = (html: string) => /<span class="embed-file">([^<]*)<\/span>/.exec(html)?.[1] ?? null;

describe("the card of an embedded file", () => {
  beforeEach(() => {
    platform.mobile = false;
    checked.length = 0;
  });

  it("on the desktop says that the file opens in another app when it does", async () => {
    const html = await embed("doc.pdf");
    expect(cardText(html)).toBe("PDF file, opens in another app.");
    expect(html).toContain('data-href="doc.pdf"');
    expect(cardText(await embed("big.log"))).toBe("LOG file, opens in another app.");
    expect(checked).toEqual(["doc.pdf", "big.log"]);
  });

  it("on the desktop says that Cairn does not open a type it refuses, or a file with no extension", async () => {
    expect(cardText(await embed("tool.exe"))).toBe("EXE file. Cairn does not open this type of file in another app.");
    expect(cardText(await embed("archive"))).toBe("File. Cairn does not open this type of file in another app.");
  });

  it("on Windows says that apps cannot open a file at a path too long for them", async () => {
    expect(cardText(await embed("deep.pdf"))).toBe("PDF file. Windows apps cannot open it at this path.");
  });

  it("on the desktop says that Cairn does not open a link to a type it refuses, though the link's own type is allowed", async () => {
    expect(cardText(await embed("looks.pdf"))).toBe("PDF file. Cairn does not open it in another app, because it links to a type of file that Cairn does not open.");
  });

  it("on the desktop says that Cairn does not open a text file marked as executable", async () => {
    expect(cardText(await embed("run.log"))).toBe("LOG file. Cairn does not open it in another app, because it is marked as executable.");
  });

  it("on the desktop names only the type when the check fails", async () => {
    const html = await embed("gone.pdf");
    expect(cardText(html)).toBe("PDF file.");
    expect(html).toContain('data-href="gone.pdf"');
  });

  it("on Android says that Cairn cannot open it in another app yet, without asking the desktop check", async () => {
    platform.mobile = true;
    const html = await embed("doc.pdf");
    expect(cardText(html)).toBe("PDF file. On Android, Cairn cannot open it in another app yet.");
    expect(html).not.toContain("opens in another app");
    expect(html).toContain('data-href="doc.pdf"');
    expect(cardText(await embed("archive"))).toBe("File. On Android, Cairn cannot open it in another app yet.");
    expect(checked).toEqual([]);
  });

  it("is not used for a small text file, which embeds as text on both", async () => {
    for (const mobile of [false, true]) {
      platform.mobile = mobile;
      const html = await embed("notes.txt");
      expect(cardText(html)).toBeNull();
      expect(html).toContain("<pre>plain text\n</pre>");
    }
    expect(checked).toEqual([]);
  });
});
