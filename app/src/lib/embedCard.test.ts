// The card of an embedded file that does not show as text (a PDF, an archive,
// a big file). On the desktop it says that the file opens in another app.
// Android cannot hand files to other apps yet (a tap on the title only shows
// a toast), so there the card says that instead.
//
// Run: cd app && npx vitest run src/lib/embedCard.test.ts

import { beforeEach, describe, expect, it, vi } from "vitest";

const platform = vi.hoisted(() => ({ mobile: false }));

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
  });

  it("on the desktop says that the file opens in another app", async () => {
    const html = await embed("doc.pdf");
    expect(cardText(html)).toBe("PDF file, opens in another app.");
    expect(html).toContain('data-href="doc.pdf"');
    expect(cardText(await embed("archive"))).toBe("File, opens in another app.");
  });

  it("on Android says that Cairn cannot open it in another app yet", async () => {
    platform.mobile = true;
    const html = await embed("doc.pdf");
    expect(cardText(html)).toBe("PDF file. On Android, Cairn cannot open it in another app yet.");
    expect(html).not.toContain("opens in another app");
    expect(html).toContain('data-href="doc.pdf"');
    expect(cardText(await embed("archive"))).toBe("File. On Android, Cairn cannot open it in another app yet.");
  });

  it("is not used for a small text file, which embeds as text on both", async () => {
    for (const mobile of [false, true]) {
      platform.mobile = mobile;
      const html = await embed("notes.txt");
      expect(cardText(html)).toBeNull();
      expect(html).toContain("<pre>plain text\n</pre>");
    }
  });
});
