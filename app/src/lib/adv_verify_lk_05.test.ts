// Reproduction for FINDING-180 (UI side): the reading view renders
// ![[pic.png]] from LinkIndex.resolve, and the core (Index::resolve, used for
// clicks and backlinks) must pick the same file. Shortest-path ties must not
// be broken by UTF-16 length/order on one side and UTF-8 bytes on the other:
// both count characters and compare code points.
// crates/cairn-core/tests/adv_verify_lk_05.rs observe_core_tie_breaks asserts
// the core's picks for the same vaults.
//
// Run: cd app && npx vitest run src/lib/adv_verify_lk_05.test.ts

import { describe, expect, it } from "vitest";
import { LinkIndex } from "./links";
import { renderUnsafe } from "./markdown";
import type { FileStat } from "./types";

const f = (path: string): FileStat => ({ path, kind: "file", size: 0, mtime: 0 });
const shown = (files: string[], raw: string) => {
  const html = renderUnsafe(raw, { links: new LinkIndex(files.map(f)), sourcePath: "x.md" });
  const m = /<img[^>]*src="vault:\/\/localhost\/([^"]*)"/.exec(html);
  return m ? m[1].split("/").map(decodeURIComponent).join("/") : null;
};

describe("FINDING-180 reading-view image embed vs core", () => {
  it("control: ASCII folders agree with the core (shortest path)", () => {
    expect(shown(["abc/pic.png", "abcd/pic.png"], "![[pic.png]]")).toBe("abc/pic.png");
  });

  it("FINDING-180: non-ASCII folder - preview shows the image the core resolves", () => {
    expect(shown(["ééé/pic.png", "abcd/pic.png"], "![[pic.png]]")).toBe("ééé/pic.png");
  });

  // Same length in characters and in UTF-16 units, so the alphabetical
  // (code point) step decides.
  it("FINDING-180: astral vs BMP folder names - alphabetical tie-break matches the core", () => {
    expect(shown(["😀ｱ/pic.png", "ｱ😀/pic.png"], "![[pic.png]]")).toBe("ｱ😀/pic.png");
  });
});
