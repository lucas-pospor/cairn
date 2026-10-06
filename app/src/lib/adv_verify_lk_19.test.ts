// Reproduction for FINDING-089 in the UI: the reading-view tag plugin
// (markdown.ts) and the Live Preview TAG_RE (livePreview.ts) must not stop at
// combining vowel signs. If they do, "#हिंदी" renders as the tag "#ह" followed
// by plain text, and clicking it searches for tag:ह.
//
//   cd app && npx vitest run src/lib/adv_verify_lk_19.test.ts

import { describe, expect, it } from "vitest";
import livePreviewSource from "./editor/livePreview.ts?raw";
import { renderUnsafe } from "./markdown";

const lpTagRe = (() => {
  const m = /const TAG_RE = \/(.*)\/([a-z]*);/.exec(livePreviewSource);
  if (!m) throw new Error("TAG_RE not found in livePreview.ts");
  return new RegExp(m[1], m[2]);
})();

const src = "notes #हिंदी and #ที่นี่";

describe("FINDING-089 (UI side)", () => {
  it("FINDING-089: reading view renders '#हिंदी' as the whole tag, not the truncated '#ह'", () => {
    const html = renderUnsafe(src, { links: null });
    const tags = [...html.matchAll(/data-tag="([^"]*)"/g)].map((m) => m[1]);
    expect(tags).toEqual(["हिंदी", "ที่นี่"]);
  });

  it("FINDING-089: Live Preview decorates all of '#हिंदी', not only '#ह'", () => {
    const tags = [...src.matchAll(lpTagRe)].map((m) => m[2]);
    expect(tags).toEqual(["हिंदी", "ที่นี่"]);
  });

  it("control: logs the tags the reading view and Live Preview produce", () => {
    const html = renderUnsafe(src, { links: null });
    const tags = [...html.matchAll(/data-tag="([^"]*)"/g)].map((m) => m[1]);
    console.log("preview tags", tags, "lp tags", [...src.matchAll(lpTagRe)].map((m) => m[2]));
  });
});
