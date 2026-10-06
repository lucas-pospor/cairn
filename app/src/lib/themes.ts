// The colour themes. Their colours are in app.css; settings.svelte.ts names the
// light and dark theme in use with data-light-theme and data-dark-theme on
// <html>, and the theme setting (system, light or dark) picks between them.
// Settings shows one Theme list: System, then every theme by name.
// The swatch colours are copies for the settings screen; themeContrast.test.ts
// checks them against app.css.

export type Scheme = "light" | "dark";
/** The "theme" setting: a scheme, or the system's choice between them. */
export type Mode = "system" | Scheme;

export interface ThemeInfo {
  /** Saved in settings.json as lightTheme or darkTheme: never rename one. */
  id: string;
  name: string;
  scheme: Scheme;
  /** --bg, --bg-side, --text and --accent, for the swatch in Settings. */
  swatch: { bg: string; side: string; text: string; accent: string };
  /**
   * The contrast its text keeps against every background it is drawn on, a
   * custom accent included: 4.5 (WCAG AA) or 7 (AAA). Borders, focus rings
   * and icons keep 3 in every theme.
   */
  minContrast: number;
}

export const THEMES: readonly ThemeInfo[] = [
  { id: "limestone", name: "Limestone", scheme: "light", minContrast: 4.5, swatch: { bg: "#fbfaf7", side: "#f3f1ec", text: "#24292b", accent: "#a84529" } },
  { id: "marble", name: "Marble", scheme: "light", minContrast: 4.5, swatch: { bg: "#ffffff", side: "#f1f4f8", text: "#1b2738", accent: "#1f5bbf" } },
  { id: "high-contrast-light", name: "High contrast light", scheme: "light", minContrast: 7, swatch: { bg: "#ffffff", side: "#f4f4f4", text: "#000000", accent: "#0b3fa8" } },
  { id: "slate", name: "Slate", scheme: "dark", minContrast: 4.5, swatch: { bg: "#1d2022", side: "#181b1d", text: "#dfe3e0", accent: "#e5774f" } },
  { id: "graphite", name: "Graphite", scheme: "dark", minContrast: 4.5, swatch: { bg: "#1e1e1e", side: "#191919", text: "#e0e0e0", accent: "#e5774f" } },
  { id: "high-contrast-dark", name: "High contrast dark", scheme: "dark", minContrast: 7, swatch: { bg: "#000000", side: "#0e0e0e", text: "#ffffff", accent: "#ffb48c" } },
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

/** The theme settings, as settings.json holds them. */
export interface ThemeSettings {
  theme: Mode;
  lightTheme?: unknown;
  darkTheme?: unknown;
}

/** The theme on screen: the light or dark one, as the mode and the system say. */
export function themeInUse(s: ThemeSettings, systemDark: boolean): ThemeInfo {
  const scheme: Scheme = s.theme === "dark" || (s.theme === "system" && systemDark) ? "dark" : "light";
  const id = themeFor(scheme, scheme === "light" ? s.lightTheme : s.darkTheme);
  return THEMES.find((t) => t.id === id)!;
}

/** What the Theme list shows: "system", or the id of the theme that light or dark uses. */
export function listedTheme(s: ThemeSettings): string {
  if (s.theme === "system") return "system";
  return themeFor(s.theme, s.theme === "light" ? s.lightTheme : s.darkTheme);
}

/**
 * The settings to save for an entry of the Theme list. System keeps the light
 * and dark theme as they are; a theme is saved as its scheme plus its id, so
 * "theme" never holds an id (versions up to 1.2.0 would replace one).
 */
export function choiceSettings(entry: string): Partial<ThemeSettings> {
  if (entry === "system") return { theme: "system" };
  const t = THEMES.find((x) => x.id === entry);
  if (!t) return {};
  return t.scheme === "light" ? { theme: "light", lightTheme: t.id } : { theme: "dark", darkTheme: t.id };
}
