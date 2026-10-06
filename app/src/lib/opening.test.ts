// Which files open where (openTarget), and which images the search panel
// lists for a query (matchImages).
//
// Run: cd app && npx vitest run src/lib/opening.test.ts

import { describe, expect, it } from "vitest";
import { openTarget } from "./opening";
import { matchImages, parseImageQuery, words } from "./imageSearch";

describe("openTarget", () => {
  const cases: [string, string, string][] = [
    // path, desktop, Android
    ["Note.md", "note", "note"],
    ["dir/Note.MARKDOWN", "note", "note"],
    ["a.png", "image", "image"],
    ["b.JPG", "image", "image"],
    ["c.jpeg", "image", "image"],
    ["d.gif", "image", "image"],
    ["e.webp", "image", "image"],
    ["f.avif", "image", "image"],
    ["g.svg", "image", "image"],
    ["h.bmp", "image", "image"],
    ["i.ico", "image", "image"],
    ["sub/dir/photo.final.PnG", "image", "image"],
    // Images the system can show but the web view may not stay with the default app.
    ["scan.tiff", "default-app", "unavailable"],
    ["photo.heic", "default-app", "unavailable"],
    ["doc.pdf", "default-app", "unavailable"],
    ["song.mp3", "default-app", "unavailable"],
    ["clip.mp4", "default-app", "unavailable"],
    ["sheet.xlsx", "default-app", "unavailable"],
    ["notes.txt", "default-app", "unavailable"],
    // The default app gets only allowed types; Rust refuses the rest (FINDING-066).
    ["run.sh", "default-app", "unavailable"],
    ["png", "default-app", "unavailable"],
    [".png", "default-app", "unavailable"],
    ["image.png.desktop", "default-app", "unavailable"],
    ["Note.md.png", "image", "image"],
  ];
  for (const [path, desktop, android] of cases) {
    it(`${path}: ${desktop} on the desktop, ${android} on Android`, () => {
      expect(openTarget(path, false)).toBe(desktop);
      expect(openTarget(path, true)).toBe(android);
    });
  }
});

describe("matchImages", () => {
  const paths = [
    "attachments/Holiday photo.png",
    "attachments/holiday-map.svg",
    "attachments/Scan.pdf",
    "Projects/Café/logo.webp",
    "Projects/Café/notes.md",
    "Projects/Café/menu.JPG",
    "İstanbul.gif",
    "diagram.png.md",
    ".hidden/secret.png",
  ];

  it("finds images whose path holds every word, case-insensitively", () => {
    expect(matchImages(paths, "holiday")).toEqual(["attachments/Holiday photo.png", "attachments/holiday-map.svg"]);
    expect(matchImages(paths, "HOLIDAY photo")).toEqual(["attachments/Holiday photo.png"]);
    expect(matchImages(paths, "attachments")).toEqual(["attachments/Holiday photo.png", "attachments/holiday-map.svg"]);
    expect(matchImages(paths, "png")).toEqual(["attachments/Holiday photo.png"]);
  });

  it("lists images only, never notes, other files or hidden files", () => {
    expect(matchImages(paths, "scan")).toEqual([]);
    expect(matchImages(paths, "notes")).toEqual([]);
    expect(matchImages(paths, "diagram")).toEqual([]);
    expect(matchImages(paths, "secret")).toEqual([]);
  });

  it("folds text as the core does (NFC and NFD, dotted capital I)", () => {
    expect(matchImages(paths, "café")).toEqual(["Projects/Café/logo.webp", "Projects/Café/menu.JPG"]);
    expect(matchImages(paths, "café menu")).toEqual(["Projects/Café/menu.JPG"]);
    expect(matchImages(paths, "istanbul")).toEqual(["İstanbul.gif"]);
  });

  it("applies path: and file: filters, also quoted ones with spaces", () => {
    expect(matchImages(paths, "path:projects")).toEqual(["Projects/Café/logo.webp", "Projects/Café/menu.JPG"]);
    expect(matchImages(paths, "file:map")).toEqual(["attachments/holiday-map.svg"]);
    expect(matchImages(paths, 'path:"holiday photo"')).toEqual(["attachments/Holiday photo.png"]);
    expect(matchImages(paths, "holiday path:svg")).toEqual(["attachments/holiday-map.svg"]);
  });

  it("finds no image for a tag or a quoted phrase, which are about a note's text", () => {
    expect(matchImages(paths, "holiday tag:trip")).toEqual([]);
    expect(matchImages(paths, "holiday #trip")).toEqual([]);
    expect(matchImages(paths, '"holiday photo"')).toEqual([]);
    expect(matchImages(paths, 'holiday "photo"')).toEqual([]);
  });

  it("needs a word or a path filter", () => {
    expect(matchImages(paths, "")).toEqual([]);
    expect(matchImages(paths, "   ")).toEqual([]);
    expect(matchImages(paths, "- . ,")).toEqual([]);
    expect(matchImages(paths, "path:")).toEqual([]);
    // An empty tag: filter is no tag (as in the core).
    expect(matchImages(paths, "tag: holiday")).toEqual(["attachments/Holiday photo.png", "attachments/holiday-map.svg"]);
  });

  it("splits words as the core does and ignores an unclosed quote", () => {
    expect(matchImages(paths, "holiday-photo")).toEqual(["attachments/Holiday photo.png"]);
    expect(matchImages(paths, 'map "')).toEqual(["attachments/holiday-map.svg"]);
    expect(parseImageQuery('a "b c" path:"d e" #t tag:x file:f')).toEqual({ words: ["a"], paths: ["d e", "f"], noteOnly: true });
  });

  it("splits words as the core's each_token does: CJK runs, emoji, enclosing marks", () => {
    // Japanese is typed without spaces between kanji, digits and Latin letters.
    expect(words("写真2024")).toEqual(["写真", "2024"]);
    expect(words("東京abc")).toEqual(["東京", "abc"]);
    expect(words("2024年")).toEqual(["2024", "年"]);
    expect(matchImages(["写真-2024.png", "写真.png", "東京/abc.png", "年報_2024.png"], "写真2024")).toEqual(["写真-2024.png"]);
    expect(matchImages(["写真-2024.png", "東京/abc.png"], "東京abc")).toEqual(["東京/abc.png"]);
    expect(matchImages(["年報_2024.png"], "2024年")).toEqual(["年報_2024.png"]);
    // Each pictograph is a word of its own (the core's is_pictograph list).
    expect(words("🇯🇵 ★✓⌘ 👍🏽")).toEqual(["🇯", "🇵", "★", "✓", "⌘", "👍", "🏽"]);
    expect(matchImages(["🇯🇵 Tokyo.png", "tokyo.png"], "🇯🇵 tokyo")).toEqual(["🇯🇵 Tokyo.png"]);
    expect(matchImages(["★ star.svg", "star.svg"], "★")).toEqual(["★ star.svg"]);
    // The keycap ends the word ("1️⃣" folds to "1" and U+20E3); an accent continues it.
    expect(words("1\u20e3")).toEqual(["1"]);
    expect(matchImages(["Photos/1.png"], "1️⃣")).toEqual(["Photos/1.png"]);
    expect(words("cafe\u0301")).toEqual(["cafe\u0301"]);
  });

  it("reads a long query in linear time", () => {
    const t0 = performance.now();
    parseImageQuery(`${"a".repeat(50000)} "x"`);
    expect(performance.now() - t0).toBeLessThan(200);
  });

  it("stops at the limit", () => {
    const many = Array.from({ length: 30 }, (_, i) => `img/p${String(i).padStart(2, "0")}.png`);
    expect(matchImages(many, "img", 10)).toEqual(many.slice(0, 10));
  });
});
