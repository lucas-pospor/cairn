// The themes as app.css defines them, for tests (themeContrast.test.ts,
// accent.test.ts). Not used by the app.

import { readFileSync } from "node:fs";
import type { AccentTheme, Rgb } from "./accent";
import { DEFAULT_THEME, THEMES } from "./themes";

// Vitest turns a CSS import into an empty string, even with ?raw.
const css = readFileSync(new URL("../app.css", import.meta.url), "utf8");

export interface Rule {
  selector: string;
  /** The enclosing at-rule, if any (e.g. "@media (prefers-color-scheme: dark)"). */
  within: string | null;
  decls: Record<string, string>;
}

/** The style rules of a stylesheet, one level of at-rule nesting deep. */
export function rules(text: string): Rule[] {
  const out: Rule[] = [];
  const src = text.replace(/\/\*[\s\S]*?\*\//g, "");
  const re = /([^{}]+)\{([^{}]*)\}|([^{}]+)\{|\}/g;
  let within: string | null = null;
  for (const m of src.matchAll(re)) {
    if (m[3] !== undefined) within = m[3].trim();
    else if (m[1] !== undefined) {
      const decls: Record<string, string> = {};
      for (const d of m[2].split(";")) {
        const i = d.indexOf(":");
        if (i > 0) decls[d.slice(0, i).trim()] = d.slice(i + 1).trim();
      }
      out.push({ selector: m[1].trim(), within, decls });
    } else within = null;
  }
  return out;
}

const RULES = rules(css);
const DARK_MEDIA = "@media (prefers-color-scheme: dark)";

/** The colour tokens of a theme rule (fonts, sizes and the shadow left out). */
const colours = (r: Rule) => Object.fromEntries(Object.entries(r.decls).filter(([, v]) => /^#[0-9a-f]{3,8}$/i.test(v)));

function rule(selector: string, within: string | null): Rule {
  const found = RULES.filter((r) => r.selector === selector && r.within === within);
  if (found.length !== 1) throw new Error(`expected one "${selector}" rule${within ? ` in ${within}` : ""} in app.css, found ${found.length}`);
  return found[0];
}

export interface Palette {
  id: string;
  name: string;
  scheme: "light" | "dark";
  /** The contrast its text must keep (themes.ts). */
  minContrast: number;
  tokens: Record<string, string>;
  /** The width and colour of its rings (--ring, --ring-color), as written. */
  ring: string | undefined;
  ringColor: string | undefined;
  /** The system-dark copy of a dark theme, which must equal `tokens` and the ring. */
  systemCopy?: Record<string, string>;
  systemRing?: string;
  systemRingColor?: string;
}

/**
 * The themes of themes.ts as app.css writes them: Limestone is the plain :root,
 * another light theme is :root:where([data-light-theme=...]); a dark theme is
 * written twice, for "System" when the system is dark and for a forced dark.
 */
export function palettes(): Palette[] {
  return THEMES.map((t) => {
    const about = { id: t.id, name: t.name, scheme: t.scheme, minContrast: t.minContrast };
    if (t.scheme === "light") {
      const r = rule(t.id === DEFAULT_THEME.light ? ":root" : `:root:where([data-light-theme="${t.id}"])`, null);
      return { ...about, tokens: colours(r), ring: r.decls["--ring"], ringColor: r.decls["--ring-color"] };
    }
    const only = t.id === DEFAULT_THEME.dark ? "" : `:where([data-dark-theme="${t.id}"])`;
    const forced = rule(`:root[data-theme="dark"]${only}`, null);
    const system = rule(`:root:not([data-theme="light"])${only}`, DARK_MEDIA);
    return {
      ...about,
      tokens: colours(forced),
      ring: forced.decls["--ring"],
      ringColor: forced.decls["--ring-color"],
      systemCopy: colours(system),
      systemRing: system.decls["--ring"],
      systemRingColor: system.decls["--ring-color"],
    };
  });
}

export function hexRgb(value: string): Rgb {
  const m = value.match(/^#([0-9a-f]{6})$/i);
  if (!m) throw new Error(`not a #rrggbb colour: ${value}`);
  return [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16)) as Rgb;
}

export const hex = (c: Rgb) => "#" + c.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("");

/** A colour expression: "--x", or "--x N% over <expression>" for color-mix(in srgb, var(--x) N%, transparent) laid over it. */
export function resolve(expr: string, tokens: Record<string, string>): Rgb {
  const m = expr.match(/^(--[\w-]+)(?: (\d+)% over (.+))?$/);
  if (!m) throw new Error(`cannot read "${expr}"`);
  if (!(m[1] in tokens)) throw new Error(`no ${m[1]} in the theme`);
  const top = hexRgb(tokens[m[1]]);
  if (m[2] === undefined) return top;
  const share = Number(m[2]) / 100;
  const below = resolve(m[3], tokens);
  return top.map((v, i) => v * share + below[i] * (1 - share)) as Rgb;
}

/** What deriveAccent needs of a theme, as settings.svelte.ts reads it from the page. */
export function accentTheme(tokens: Record<string, string>): AccentTheme {
  const t = (name: string) => hexRgb(tokens[name]);
  return { bg: t("--bg"), side: t("--bg-side"), hover: t("--bg-hover"), code: t("--bg-code"), soft: t("--accent-soft"), hit: t("--hit") };
}
