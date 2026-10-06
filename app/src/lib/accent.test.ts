import { describe, expect, it } from "vitest";
import { contrast, deriveAccent, parseRgb, type AccentTheme, type Rgb } from "./accent";
import { accentTheme, palettes } from "./themes.testutil";

const rgb = (hex: string): Rgb => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Rgb;

// The themes from app.css.
const THEMES: Record<string, AccentTheme> = Object.fromEntries(palettes().map((p) => [p.id, accentTheme(p.tokens)]));
const under = (hit: Rgb, bg: Rgb) => hit.map((v, i) => v * 0.45 + bg[i] * 0.55) as Rgb;
/** Where accent text is drawn: page, sidebar, hovered row, code, the tint, and under a selection match. */
const surfaces = (theme: AccentTheme, soft: Rgb) => [theme.bg, theme.side, theme.hover, theme.code, soft, under(theme.hit, theme.bg), under(theme.hit, soft)];

describe("deriveAccent (FINDING-219)", () => {
  for (const [name, theme] of Object.entries(THEMES)) {
    for (const accent of ["#3b82f6", "#e8a33d", "#ffff00", "#ffffff", "#000000", "#ff0000", "#a84529", "#e5774f"]) {
      it(`${accent} in the ${name} theme: accent text, button text and the tint stay readable`, () => {
        const d = deriveAccent(rgb(accent), theme);
        const [c, text, soft] = [rgb(d.accent), rgb(d.text), rgb(d.soft)];
        // Links and tags on every background they are drawn on.
        for (const bg of surfaces(theme, soft)) expect(contrast(c, bg)).toBeGreaterThanOrEqual(4.5);
        // Primary button text.
        expect(contrast(text, c)).toBeGreaterThanOrEqual(4.5);
        // The tint is no further from --bg than the theme's own, so text that reads on that reads on this.
        expect(contrast(soft, theme.bg)).toBeLessThanOrEqual(contrast(theme.soft, theme.bg));
      });
    }
  }

  it("every colour on a coarse grid stays readable in every theme", () => {
    const levels = [0, 37, 73, 110, 146, 183, 219, 255];
    const bad: string[] = [];
    for (const [name, theme] of Object.entries(THEMES))
      for (const r of levels)
        for (const g of levels)
          for (const b of levels) {
            const d = deriveAccent([r, g, b], theme);
            const [c, text, soft] = [rgb(d.accent), rgb(d.text), rgb(d.soft)];
            const worst = Math.min(...surfaces(theme, soft).map((bg) => contrast(c, bg)), contrast(text, c));
            if (worst < 4.5) bad.push(`${name} rgb(${r}, ${g}, ${b}): ${worst.toFixed(2)}`);
          }
    expect(bad).toEqual([]);
  });

  it("keeps an accent that is already readable", () => {
    expect(deriveAccent(rgb("#a84529"), THEMES.limestone).accent).toBe("#a84529");
    expect(deriveAccent(rgb("#e8a33d"), THEMES.slate).accent).toBe("#e8a33d");
  });

  it("shades in a light theme and lightens in a dark theme", () => {
    const blue = rgb("#3b82f6");
    const light = rgb(deriveAccent(blue, THEMES.limestone).accent);
    const dark = rgb(deriveAccent(blue, THEMES.slate).accent);
    expect(light.every((v, i) => v < blue[i])).toBe(true);
    expect(dark.every((v, i) => v > blue[i])).toBe(true);
  });

  it("parses computed colours", () => {
    expect(parseRgb("rgb(59, 130, 246)")).toEqual([59, 130, 246]);
    expect(parseRgb("rgba(1, 2, 3, 0.5)")).toEqual([1, 2, 3]);
    expect(parseRgb("rgb(1 2 3)")).toEqual([1, 2, 3]);
    expect(parseRgb("color(srgb 1 0 0)")).toBeNull();
  });
});
