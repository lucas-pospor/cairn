// The checks on a font file (textFont.ts) before Cairn saves or loads it: its
// name, its first bytes and its size.
//
// Run: cd app && npx vitest run src/lib/textFont.test.ts

import { describe, expect, it } from "vitest";
import { MAX_FONT_BYTES, fontBytesProblem, fontFailure, fontFormat, fontNameProblem, fontSizeProblem } from "./textFont";

/** A file that starts with `head`, padded to `size` bytes. */
const file = (head: number[] | string, size = 64) => {
  const b = new Uint8Array(size);
  b.set(typeof head === "string" ? [...head].map((c) => c.charCodeAt(0)) : head);
  return b;
};

describe("font file names", () => {
  it("takes woff2, woff, ttf and otf files, in any case", () => {
    for (const name of ["Inter.woff2", "Inter.woff", "Noto Serif CJK.ttf", "Lora-Italic.otf", "SHOUT.TTF", "a.b.Woff2"]) expect(fontNameProblem(name), name).toBeNull();
  });

  it("refuses other types, folders, dot files and values that are not a name", () => {
    for (const name of ["Inter.ttc", "Inter.svg", "Inter.eot", "Inter", "ttf", "Inter.ttf.zip"]) expect(fontNameProblem(name), name).toBe("Cairn takes woff2, woff, ttf and otf font files.");
    for (const name of ["fonts/Inter.ttf", "..\\Inter.ttf", "../Inter.ttf", ".Inter.ttf", "a\0.ttf"]) expect(fontNameProblem(name), name).toBe("A font file name cannot contain slashes or start with a dot.");
    for (const name of [undefined, null, 5, ["Inter.ttf"], { file: "Inter.ttf" }, "", "  "]) expect(fontNameProblem(name), String(name)).toBe("The font has no file name.");
  });
});

describe("font file contents", () => {
  it("knows each format by its first bytes", () => {
    expect(fontFormat(file("wOF2"))).toBe("woff2");
    expect(fontFormat(file("wOFF"))).toBe("woff");
    expect(fontFormat(file("OTTO"))).toBe("opentype");
    expect(fontFormat(file([0, 1, 0, 0]))).toBe("truetype");
    expect(fontFormat(file("true"))).toBe("truetype");
    expect(fontFormat(file("ttcf"))).toBe("collection");
    for (const head of ["<svg", "PK\u0003\u0004", "%PDF", "wof2", "\u0000\u0001\u0000\u0001"]) expect(fontFormat(file(head)), head).toBeNull();
    // Too short to be a font, whatever it starts with.
    expect(fontFormat(file("wOF2", 8))).toBeNull();
  });

  it("explains what is wrong with a file that is not a font Cairn takes", () => {
    expect(fontBytesProblem(file("wOF2"))).toBeNull();
    expect(fontBytesProblem(file([0, 1, 0, 0]))).toBeNull();
    expect(fontBytesProblem(file("ttcf"))).toBe("Font collections (ttc) are not supported. Choose a woff2, woff, ttf or otf file.");
    expect(fontBytesProblem(file("%PDF"))).toBe("This is not a woff2, woff, ttf or otf font file.");
    expect(fontBytesProblem(new Uint8Array(0))).toBe("This is not a woff2, woff, ttf or otf font file.");
  });

  it("takes files up to 20 MB", () => {
    expect(MAX_FONT_BYTES).toBe(20 * 1024 * 1024);
    expect(fontSizeProblem(MAX_FONT_BYTES)).toBeNull();
    expect(fontSizeProblem(MAX_FONT_BYTES + 1)).toBe("The font file is 21 MB; Cairn takes font files up to 20 MB.");
    expect(fontSizeProblem(64 * 1024 * 1024)).toBe("The font file is 64 MB; Cairn takes font files up to 20 MB.");
  });
});

describe("why a font file could not be used", () => {
  it("says it in words, without an error's name", () => {
    expect(fontFailure({ kind: "notFound", detail: "fonts/Inter.woff2" })).toBe("It is not in .cairn/fonts/.");
    expect(fontFailure({ kind: "io", detail: "\"fonts/Big.ttf\" is 30 MB, more than the 20 MB that Cairn reads." })).toBe("\"fonts/Big.ttf\" is 30 MB, more than the 20 MB that Cairn reads.");
    expect(fontFailure(new Error("A network error occurred."))).toBe("A network error occurred.");
    expect(fontFailure("plain")).toBe("plain");
  });
});
