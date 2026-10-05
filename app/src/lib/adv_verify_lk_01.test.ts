// Reproduction for FINDING-035 on the TypeScript side: LinkIndex
// (used for unresolved styling and image embeds) must resolve `../` and `./`.
// Run: cd app && npx vitest run src/lib/adv_verify_lk_01.test.ts

import { describe, expect, it } from "vitest";
import { LinkIndex } from "./links";
import { renderUnsafe } from "./markdown";
import type { FileStat } from "./types";

const f = (path: string): FileStat => ({ path, kind: "file", size: 0, mtime: 0 }) as FileStat;
const idx = new LinkIndex([f("Top.md"), f("a/Sibling.md"), f("a/sub/Child.md"), f("b/Target.md"), f("attachments/pic.png"), f("a/src.md")]);

describe("FINDING-035 (TypeScript LinkIndex)", () => {
  it("control: descendant relative wikilink and plain names resolve", () => {
    expect(idx.resolve("sub/Child", "a/src.md")).toBe("a/sub/Child.md");
    expect(idx.exists("sub/Child")).toBe(true);
    expect(idx.resolve("Top", "a/src.md")).toBe("Top.md");
  });

  it("FINDING-035: [[../Top]] and [[./Sibling]] resolve and are not styled unresolved", () => {
    expect(idx.resolve("../Top", "a/src.md")).toBe("Top.md");
    expect(idx.resolve("./Sibling", "a/src.md")).toBe("a/Sibling.md");
    expect(idx.resolve("../b/Target", "a/src.md")).toBe("b/Target.md");
    expect(idx.exists("../Top")).toBe(true);
  });

  it("FINDING-035: ![[../attachments/pic.png]] renders as an image in reading view", () => {
    const html = renderUnsafe("![[../attachments/pic.png]]\n\n[[../Top]]\n", { links: idx, sourcePath: "a/src.md" });
    expect(html).toContain('<img class="embed-image"');
    expect(html).not.toContain("is-unresolved");
  });
});
