// A custom accent (Settings > Appearance) is used as link and tag text, as the
// fill of primary buttons and, as a faint tint, behind tags and selected rows.
// Taken as-is, many colours are unreadable there, so the accent tokens are
// derived from it: the colour is shaded (light theme) or lightened (dark theme)
// only as far as WCAG AA (4.5:1) needs, button text is white or a near-black of
// the same hue, and the tint stays no further from --bg than the theme's own
// tint, so text that reads on the theme's tint also reads on it.

export type Rgb = [number, number, number];

/** A computed colour ("rgb(r, g, b)" or "rgba(...)"); null for any other form. */
export function parseRgb(css: string): Rgb | null {
  const m = css.trim().match(/^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/);
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

const channel = (v: number) => {
  const c = v / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
};
const luminance = ([r, g, b]: Rgb) => 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);

/** WCAG contrast ratio. */
export function contrast(a: Rgb, b: Rgb): number {
  const x = luminance(a), y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** `share` of `a` over `b`, rounded to whole channels so the checks below see the colour that is used. */
const mix = (a: Rgb, b: Rgb, share: number) => a.map((v, i) => Math.round(v * share + b[i] * (1 - share))) as Rgb;
const hex = (c: Rgb) => "#" + c.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("");
const WHITE: Rgb = [255, 255, 255], BLACK: Rgb = [0, 0, 0];

/** The theme's --bg, --bg-side and its own --accent-soft. */
export interface AccentTheme {
  bg: Rgb;
  side: Rgb;
  soft: Rgb;
}

/** Values for --accent (and --link), --accent-text and --accent-soft. */
export function deriveAccent(accent: Rgb, theme: AccentTheme): { accent: string; text: string; soft: string } {
  const light = contrast(theme.bg, BLACK) > contrast(theme.bg, WHITE);
  // The tint: as much of the accent (up to 22%) as keeps it no further from --bg than the theme's tint.
  const softLimit = contrast(theme.soft, theme.bg);
  const tint = (c: Rgb) => {
    let pct = 22;
    while (pct > 2 && contrast(mix(c, theme.bg, pct / 100), theme.bg) > softLimit) pct--;
    return mix(c, theme.bg, pct / 100);
  };
  const readable = (c: Rgb, soft: Rgb) => Math.min(contrast(c, theme.bg), contrast(c, theme.side), contrast(c, soft)) >= 4.5;
  let c = accent;
  let soft = tint(c);
  for (let step = 1; !readable(c, soft) && step <= 50; step++) {
    c = mix(light ? BLACK : WHITE, accent, step / 50);
    soft = tint(c);
  }
  const ink = mix(c, BLACK, 0.12);
  return { accent: hex(c), text: contrast(WHITE, c) >= contrast(ink, c) ? "#ffffff" : hex(ink), soft: hex(soft) };
}
