// Reproduction for FINDING-086.
// Run: cd app && npx vitest run src/lib/adv_verify_lk_12.test.ts
//
// LinkIndex.exists() is case-insensitive and accepts a suffix match, so it is
// true for paths that do not exist. An image rule that used the plain relative
// path whenever exists() is true would never reach the fallback to
// LinkIndex.resolve() (which finds the real file), and the vault:// handler,
// which reads the exact path, would answer with a 404. markdown.ts and
// livePreview.ts use the relative path only when LinkIndex.has() it.

import { describe, expect, it } from "vitest";
import { LinkIndex } from "./links";
import { renderUnsafe } from "./markdown";
import type { FileStat } from "./types";

const idx = (files: string[]) =>
  new LinkIndex(files.map((path): FileStat => ({ path, kind: "file", size: 1, mtime: 0 })));

function imgSrc(raw: string, links: LinkIndex, sourcePath: string): string | null {
  const m = /<img[^>]*src="vault:\/\/localhost\/([^"]*)"/.exec(renderUnsafe(raw, { links, sourcePath }));
  return m ? m[1].split("/").map(decodeURIComponent).join("/") : null;
}

describe("FINDING-086", () => {
  it("controls: exact relative path and the TS resolver both give the real file", () => {
    const l = idx(["a/img/pic.png", "a/src.md", "z/a/pic.png", "n.md"]);
    expect(imgSrc("![c](img/pic.png)", l, "a/src.md")).toBe("a/img/pic.png");
    // The resolver that markdown.ts falls back to already finds the real files:
    expect(l.resolve("img/Pic.png", "a/src.md")).toBe("a/img/pic.png");
    expect(l.resolve("a/pic.png", "n.md")).toBe("z/a/pic.png");
    // ...but exists() says yes for paths that are not in the vault:
    expect(l.exists("a/img/Pic.png")).toBe(true);
    expect(l.exists("a/pic.png")).toBe(true);
  });

  it("FINDING-086: case-only mismatch loads the real file, not a/img/Pic.png", () => {
    const l = idx(["a/img/pic.png", "a/src.md"]);
    expect(imgSrc("![case](img/Pic.png)", l, "a/src.md")).toBe("a/img/pic.png");
  });

  it("FINDING-086: suffix-only match loads the real file, not a/pic.png", () => {
    const l = idx(["z/a/pic.png", "n.md"]);
    expect(imgSrc("![x](a/pic.png)", l, "n.md")).toBe("z/a/pic.png");
  });
});
