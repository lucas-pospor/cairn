// WCAG contrast of every theme in app.css. Each foreground below is listed with
// the backgrounds it is drawn on somewhere in the app; text needs 4.5:1, and
// input borders, focus rings and icons 3:1. A background written "--x N% over --y"
// is color-mix(in srgb, var(--x) N%, transparent) laid over --y, as the CSS does.
// Colours that CodeMirror and sigma draw themselves are not covered here.
//
// Run: cd app && npx vitest run src/lib/themeContrast.test.ts

import { describe, expect, it } from "vitest";
import { contrast, deriveAccent, type Rgb } from "./accent";
import { accentTheme, hex, hexRgb, palettes, resolve, rules } from "./themes.testutil";

/** Text: foreground -> background -> where. */
const TEXT: Record<string, Record<string, string>> = {
  "--text": {
    "--bg": "notes, dialogs, settings",
    "--bg-side": "sidebars, tab bar, status bar",
    "--bg-input": "inputs, menus, toasts",
    "--bg-hover": "buttons, hovered rows",
    "--bg-active": "pressed buttons, the open file's row",
    "--bg-code": "inline code and code blocks",
    "--accent-soft": "selected text, selected rows",
    "--hit": "a search highlight",
    "--hit 45% over --bg": "other matches of the selected word",
    "--hit 45% over --bg-code": "other matches of the selected word in code",
    "--bg-side 70% over --bg": "an embedded note",
    "--bg-side 70% over --bg-side 70% over --bg": "an embed inside an embed",
    "--danger 12% over --bg": "the conflict banner",
  },
  "--text-muted": {
    "--bg": "hints, quotes, graph toolbar",
    "--bg-side": "status bar, folder names, sidebar hints",
    "--bg-hover": "hovered rows",
    "--bg-code": "a quote in code",
    "--accent-soft": "selected rows, selected quotes",
    "--hit": "a search result's matched words",
    "--hit 45% over --bg": "a selection match in a quote",
    "--hit 45% over --bg-code": "a selection match in quoted code",
    "--bg-side 70% over --bg": "an embed's title",
  },
  "--text-faint": {
    "--bg": "placeholders, Markdown marks, URLs",
    "--bg-side": "chevrons, tab close buttons",
    "--bg-input": "input placeholders",
    "--bg-code": "code fences and backticks",
    "--accent-soft": "selected marks and URLs",
    "--hit 45% over --bg": "a selection match on marks",
    "--hit 45% over --bg-code": "a selection match on code fences and backticks",
    "--bg-side 70% over --bg": "a missing embed",
    "--bg-side 70% over --bg-side 70% over --bg-side 70% over --bg-side 70% over --bg": "an embed nested too deep",
  },
  "--accent": {
    "--bg": "tags, the graph's active note",
    "--bg-side": "pressed panel buttons, syncing state",
    "--bg-hover": "pressed buttons and tags when hovered",
    "--accent-soft": "tags",
    "--hit 45% over --accent-soft": "a selection match in a tag",
  },
  "--link": {
    "--bg": "links",
    "--bg-side": "outgoing links panel",
    "--bg-hover": "hovered outgoing links",
    "--bg-code": "code inside a link",
    "--accent-soft": "selected links",
    "--hit 45% over --bg": "a selection match in a link",
    "--bg-side 70% over --bg": "links in an embed",
  },
  "--unresolved": {
    "--bg": "links to missing notes",
    "--bg-code": "code inside a link to a missing note",
    "--bg-side": "outgoing links panel",
    "--bg-hover": "hovered outgoing links",
    "--accent-soft": "selected links",
    "--hit 45% over --bg": "a selection match in a link",
    "--bg-side 70% over --bg": "links in an embed",
  },
  "--accent-text": { "--accent": "primary buttons" },
  "--danger-text": { "--danger": "Delete buttons" },
  "--danger": {
    "--bg": "error messages",
    "--bg-side": "sync error in the status bar",
    "--bg-hover": "sync error, hovered",
    "--bg-input": "error toasts, Delete in menus",
    "--accent-soft": "Delete in menus, focused",
    "--danger 20% over --bg": "the sync error in Settings",
  },
};
for (const tok of ["--tok-keyword", "--tok-string", "--tok-number", "--tok-comment", "--tok-fn", "--tok-type"])
  TEXT[tok] = {
    "--bg": "highlighted front matter",
    "--bg-code": "code blocks",
    "--accent-soft": "selected code",
    "--hit 45% over --bg-code": "a selection match in code",
  };

/** Input borders, focus rings and icons: foreground -> background -> where. */
const UI: Record<string, Record<string, string>> = {
  "--border-strong": {
    "--bg": "input borders",
    "--bg-side": "input borders in sidebars",
    "--bg-input": "input borders, inner side",
    "--bg-hover": "the ring of the selected file row, hovered",
  },
  "--accent": {
    "--bg-input": "focused input border",
    "--bg-code": "the caret in code",
    "--bg-active": "keyboard cursor on the open file's row",
  },
  "--text-muted": { "--bg-input": "close button on toasts" },
  "--text-faint": { "--bg-hover": "chevrons on hovered rows" },
};

/** Contrast of a primary button's label while hovered (filter: brightness(1.06)). */
function hoveredButton(tokens: Record<string, string>): number {
  const bright = (v: string) => hexRgb(tokens[v]).map((c) => Math.min(255, c * 1.06)) as Rgb;
  return contrast(bright("--accent-text"), bright("--accent"));
}

/** Every pair below its minimum, as "fg on bg (where): ratio". */
function failures(tokens: Record<string, string>, only?: (fg: string, bg: string) => boolean): string[] {
  const bad: string[] = [];
  for (const [table, min] of [[TEXT, 4.5], [UI, 3]] as const)
    for (const [fg, bgs] of Object.entries(table))
      for (const [bg, where] of Object.entries(bgs)) {
        if (only && !only(fg, bg)) continue;
        const ratio = contrast(resolve(fg, tokens), resolve(bg, tokens));
        if (ratio < min) bad.push(`${fg} on ${bg} (${where}): ${ratio.toFixed(2)}, needs ${min}`);
      }
  return bad;
}

describe("theme contrast", () => {
  for (const p of palettes()) {
    it(`the ${p.name} theme sets every colour, once`, () => {
      expect(Object.keys(p.tokens).sort()).toEqual(Object.keys(palettes()[0].tokens).sort());
      if (p.systemCopy) expect(p.systemCopy, "the copy used when the system is dark").toEqual(p.tokens);
    });

    it(`the ${p.name} theme meets WCAG AA`, () => {
      expect(failures(p.tokens)).toEqual([]);
    });

    it(`the ${p.name} theme: a primary button stays readable when hovered (brightness 1.06)`, () => {
      expect(hoveredButton(p.tokens)).toBeGreaterThanOrEqual(4.5);
    });

    it(`the ${p.name} theme: a custom accent is adjusted until it meets WCAG AA wherever the accent is used`, () => {
      const levels = [0, 37, 73, 110, 146, 183, 219, 255];
      const bad: string[] = [];
      for (const r of levels)
        for (const g of levels)
          for (const b of levels) {
            const d = deriveAccent([r, g, b], accentTheme(p.tokens));
            const tokens = { ...p.tokens, "--accent": d.accent, "--link": d.accent, "--accent-text": d.text, "--accent-soft": d.soft };
            const fails = failures(tokens, (fg, bg) => /--accent|--link/.test(`${fg} ${bg}`));
            if (hoveredButton(tokens) < 4.5) fails.push("hovered primary button");
            bad.push(...fails.map((f) => `${hex([r, g, b])}: ${f}`));
          }
      expect(bad).toEqual([]);
    });
  }

  it("reads the expressions it checks", () => {
    const t = { "--a": "#000000", "--b": "#ffffff" };
    expect(resolve("--a 50% over --b", t)).toEqual([127.5, 127.5, 127.5]);
    expect(resolve("--a 50% over --a 50% over --b", t)).toEqual([63.75, 63.75, 63.75]);
    expect(rules("@media x { a { b: c; } } d { e: f }")).toEqual([
      { selector: "a", within: "@media x", decls: { b: "c" } },
      { selector: "d", within: null, decls: { e: "f" } },
    ]);
  });
});

