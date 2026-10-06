// The colour themes. Their colours are in app.css; settings.svelte.ts names the
// light and dark theme in use with data-light-theme and data-dark-theme on
// <html>, and the Theme setting (system, light or dark) picks between them.
// The swatch colours are copies for the settings screen; themeContrast.test.ts
// checks them against app.css.

export type Scheme = "light" | "dark";

export interface ThemeInfo {
  /** Saved in settings.json as lightTheme or darkTheme: never rename one. */
  id: string;
  name: string;
  scheme: Scheme;
  /** --bg, --bg-side, --text and --accent, for the swatch in Settings. */
  swatch: { bg: string; side: string; text: string; accent: string };
}

export const THEMES: readonly ThemeInfo[] = [
  { id: "limestone", name: "Limestone", scheme: "light", swatch: { bg: "#fbfaf7", side: "#f3f1ec", text: "#24292b", accent: "#a84529" } },
  { id: "marble", name: "Marble", scheme: "light", swatch: { bg: "#ffffff", side: "#f1f4f8", text: "#1b2738", accent: "#1f5bbf" } },
  { id: "slate", name: "Slate", scheme: "dark", swatch: { bg: "#1d2022", side: "#181b1d", text: "#dfe3e0", accent: "#e5774f" } },
  { id: "graphite", name: "Graphite", scheme: "dark", swatch: { bg: "#1e1e1e", side: "#191919", text: "#e0e0e0", accent: "#e5774f" } },
];

/** The themes used when none is chosen; versions before these settings have only these two. */
export const DEFAULT_THEME: Record<Scheme, string> = { light: "limestone", dark: "slate" };

/**
 * The theme to show for `scheme`: the saved one if this version has it, else
 * the default (for an id from a later version, or a hand edit).
 */
export function themeFor(scheme: Scheme, saved: unknown): string {
  return THEMES.some((t) => t.scheme === scheme && t.id === saved) ? (saved as string) : DEFAULT_THEME[scheme];
}
