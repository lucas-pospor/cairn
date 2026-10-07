// Per-vault settings, stored in `<vault>/.cairn/settings.json` so they travel
// with the vault. Custom CSS snippets live in `<vault>/.cairn/snippets/`.

import { backend } from "./backend";
import { deriveAccent, parseRgb } from "./accent";
import { themeFor, themeInUse } from "./themes";
import { FONT_DIR, MAX_FONT_BYTES, fontBytesProblem, fontFailure, fontNameProblem, loadFontFace } from "./textFont";

export type Theme = "system" | "light" | "dark";

export interface Settings {
  /** Light, dark, or the system's choice between them. */
  theme: Theme;
  /**
   * The light and dark theme ids (themes.ts). Kept as read, whatever they
   * hold, and not written until chosen: an id this version does not know shows
   * the default theme and stays in the file. Older versions keep these keys
   * too, but replace a "theme" they do not know, so it never holds an id.
   */
  lightTheme?: unknown;
  darkTheme?: unknown;
  accent: string;
  fontSize: number;
  lineWidth: number;
  /** The Text font; also used wherever the font file (textFont) cannot load. */
  fontFamily: "sans" | "serif" | "mono";
  /**
   * The file name of the user's font for note text in `.cairn/fonts/`
   * (textFont.ts). Kept as read and not written until chosen, like the theme
   * ids: versions up to 1.2.0 keep it and show fontFamily, which is why the
   * font file has a key of its own.
   */
  textFont?: unknown;
  defaultMode: "live" | "source" | "preview";
  attachmentFolder: string;
  /** Enabled CSS snippet file names. */
  snippets: string[];
  /** Command id -> key combos, overriding the defaults. */
  hotkeys: Record<string, string[]>;
  graphShowUnresolved: boolean;
  spellcheck: boolean;
  /** Enabled plugin files in `.cairn/plugins`. */
  plugins: string[];
  /**
   * Core plugins: the switches and options the user changed, by plugin id (see
   * corePlugins/core.ts). Kept as read, whatever it holds, and not written until set.
   */
  corePlugins?: unknown;
}

export const DEFAULT_SETTINGS: Settings = {
  theme: "system",
  accent: "",
  fontSize: 16,
  lineWidth: 760,
  fontFamily: "sans",
  defaultMode: "live",
  attachmentFolder: "attachments",
  snippets: [],
  hotkeys: {},
  graphShowUnresolved: false,
  spellcheck: false,
  plugins: [],
};

/**
 * The hotkey overrides of a settings.json, which may have been edited by
 * hand: only lists of key combos count, so anything else keeps a command's
 * default keys (and is named in a console warning).
 */
export function validHotkeys(v: unknown): Record<string, string[]> {
  if (v === undefined) return {};
  if (!v || typeof v !== "object" || Array.isArray(v)) {
    console.warn("settings.json: ignoring \"hotkeys\", which is not an object");
    return {};
  }
  const entries = Object.entries(v);
  const ok = entries.filter(([, keys]) => Array.isArray(keys) && keys.every((k) => typeof k === "string"));
  if (ok.length < entries.length) {
    const bad = entries.filter((e) => !ok.includes(e)).map(([id]) => id);
    console.warn(`settings.json: ignoring hotkeys that are not lists of keys: ${bad.join(", ")}`);
  }
  return Object.fromEntries(ok);
}

const FONTS: Record<Settings["fontFamily"], string> = {
  sans: "var(--font-ui)",
  serif: 'Charter, "Iowan Old Style", "Source Serif 4", Georgia, "Noto Serif", serif',
  mono: "var(--font-mono)",
};
/** The Text font choices by name, as Settings shows them. */
export const FONT_NAMES: Record<Settings["fontFamily"], string> = { sans: "Sans serif", serif: "Serif", mono: "Monospace" };

/** The font file named by textFont: loading, in use, or why it is not. */
export interface FontFileState {
  name: string;
  state: "loading" | "loaded" | "failed";
  error?: string;
}

function oneOf<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
  return allowed.includes(value as T) ? (value as T) : fallback;
}

class SettingsStore {
  value = $state<Settings>({ ...DEFAULT_SETTINGS });
  /** Snippet files present in `.cairn/snippets`. */
  available = $state<string[]>([]);
  private saveTimer: ReturnType<typeof setTimeout> | undefined;
  /** A change not written to settings.json yet. */
  private dirty = false;
  /** The last write of settings.json failed, and no write has worked since. */
  private failed = false;
  private writing: Promise<void> = Promise.resolve();
  private loaded = false;
  /** Told when a change could not be saved after the debounce; the app shows a toast. */
  onSaveError: (e: unknown) => void = (e) => console.warn("settings.json not saved", e);
  /** The font file named by textFont, or null when none is (or it is not a file name at all). */
  font = $state<FontFileState | null>(null);
  private face: FontFace | null = null;
  /** Counts font loads, so one that finishes after another has started is dropped. */
  private fontGen = 0;
  /** Told when the font file cannot be used; the app shows a toast. */
  onFontError: (name: string, message: string) => void = (name, message) => console.warn(`font file ${name} not loaded: ${message}`);
  /** Told when the note text font changed after a font file loaded or went; the editor measures its lines again. */
  onFontChange: () => void = () => {};

  constructor() {
    // The derived accent depends on the theme, which follows the system with theme "system".
    if (typeof matchMedia === "function") matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => this.applyAccent());
  }

  async load() {
    clearTimeout(this.saveTimer);
    this.dirty = false;
    this.failed = false;
    this.loaded = false;
    let data: Partial<Settings> = {};
    try {
      const raw = await backend.readConfig("settings.json");
      const parsed: unknown = raw ? JSON.parse(raw) : {};
      // Valid JSON that is not an object (null, a list, a number) holds no settings.
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) data = parsed as Partial<Settings>;
      else console.warn("settings.json is not a JSON object, using defaults");
    } catch (e) {
      console.warn("settings.json unreadable, using defaults", e);
    }
    const value = { ...DEFAULT_SETTINGS, ...data, hotkeys: validHotkeys(data.hotkeys) };
    // A hand edit or another version may hold a choice this build does not have.
    value.theme = oneOf(value.theme, ["system", "light", "dark"], DEFAULT_SETTINGS.theme);
    value.fontFamily = oneOf(value.fontFamily, ["sans", "serif", "mono"], DEFAULT_SETTINGS.fontFamily);
    value.defaultMode = oneOf(value.defaultMode, ["live", "source", "preview"], DEFAULT_SETTINGS.defaultMode);
    this.value = value;
    if (value.textFont !== undefined && typeof value.textFont !== "string") console.warn("settings.json: \"textFont\" is not a file name, so the Text font is used");
    // Another vault's font file may have the same name.
    this.dropFont();
    await this.refreshSnippets();
    this.loaded = true;
    this.apply();
  }

  async refreshSnippets() {
    try {
      this.available = (await backend.listConfig("snippets")).filter((n) => n.endsWith(".css"));
    } catch {
      this.available = [];
    }
  }

  /** Change settings and persist (debounced). */
  update(patch: Partial<Settings>) {
    this.value = { ...this.value, ...patch };
    this.apply();
    if (!this.loaded) return;
    this.dirty = true;
    clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => {
      this.flush().catch((e) => this.onSaveError(e));
    }, 300);
  }

  /** Write a pending change now instead of after the debounce. Throws if the write fails. */
  flush(): Promise<void> {
    clearTimeout(this.saveTimer);
    // One write at a time, so an older value never lands after a newer one.
    const run = this.writing.then(async () => {
      while (this.dirty) {
        const value = this.value;
        try {
          await backend.writeConfig("settings.json", JSON.stringify(value, null, 2) + "\n");
        } catch (e) {
          this.failed = true;
          throw e;
        }
        this.failed = false;
        if (this.value === value) this.dirty = false;
      }
    });
    this.writing = run.catch(() => {});
    return run;
  }

  /** A change whose write failed is not saved yet; one that waits for the debounce does not count. */
  get saveFailed() {
    return this.failed;
  }

  async apply() {
    const s = this.value;
    const root = document.documentElement;
    if (s.theme === "system") root.removeAttribute("data-theme");
    else root.dataset.theme = s.theme;
    root.dataset.lightTheme = themeFor("light", s.lightTheme);
    root.dataset.darkTheme = themeFor("dark", s.darkTheme);
    root.style.setProperty("--text-size", `${s.fontSize}px`);
    root.style.setProperty("--line-width", `${s.lineWidth}px`);
    root.style.setProperty("--font-text", this.fontStack());
    void this.syncFont();
    this.applyAccent();
    await this.applySnippets();
  }

  /** --font-text: the font file once it is loaded, with the Text font behind it. */
  private fontStack(): string {
    const fallback = FONTS[this.value.fontFamily] ?? FONTS.sans;
    return this.face && this.font?.state === "loaded" && this.font.name === this.value.textFont ? `"${this.face.family}", ${fallback}` : fallback;
  }

  /** Load the font file that textFont names, unless it is the one loaded (or failing) already; `again` loads it anyway. */
  private async syncFont(again = false) {
    const want = this.value.textFont;
    const name = typeof want === "string" ? want : null;
    if (!again && (this.font?.name ?? null) === name) return;
    const had = this.face !== null;
    this.dropFont();
    const gen = this.fontGen;
    const root = document.documentElement;
    if (name === null) {
      if (had) {
        root.style.setProperty("--font-text", this.fontStack());
        this.onFontChange();
      }
      return;
    }
    this.font = { name, state: "loading" };
    try {
      const problem = fontNameProblem(name);
      if (problem) throw new Error(problem);
      const bytes = await backend.readConfigBytes(`${FONT_DIR}/${name}`, MAX_FONT_BYTES);
      if (gen !== this.fontGen) return;
      const bad = fontBytesProblem(new Uint8Array(bytes));
      if (bad) throw new Error(bad);
      const face = await loadFontFace(bytes);
      if (gen !== this.fontGen) return;
      document.fonts.add(face);
      this.face = face;
      this.font = { name, state: "loaded" };
    } catch (e) {
      if (gen !== this.fontGen) return;
      const message = fontFailure(e);
      this.font = { name, state: "failed", error: message };
      this.onFontError(name, message);
    }
    root.style.setProperty("--font-text", this.fontStack());
    this.onFontChange();
  }

  /** Load the font file again: it was replaced by one of the same name. */
  reloadFont(): Promise<void> {
    return this.syncFont(true);
  }

  /** Stop using the font file (a load under way is dropped too). */
  private dropFont() {
    this.fontGen++;
    if (this.face) document.fonts?.delete(this.face);
    this.face = null;
    this.font = null;
  }

  /** The custom accent, adjusted to the current theme so text in and on it stays readable (see accent.ts). */
  private applyAccent() {
    const root = document.documentElement;
    for (const v of ["--accent", "--link", "--accent-text", "--accent-soft"]) root.style.removeProperty(v);
    const accent = this.value.accent;
    if (!accent || !CSS.supports("color", accent)) return;
    // Resolve colours as the browser computes them, with the theme's own accent tokens in place.
    const probe = document.createElement("span");
    document.body.appendChild(probe);
    const rgb = (css: string) => {
      probe.style.color = css;
      return parseRgb(getComputedStyle(probe).color);
    };
    const [c, bg, side, hover, code, hit, soft] = [accent, "var(--bg)", "var(--bg-side)", "var(--bg-hover)", "var(--bg-code)", "var(--hit)", "var(--accent-soft)"].map(rgb);
    probe.remove();
    if (!c || !bg || !side || !hover || !code || !hit || !soft) return;
    const systemDark = typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;
    const d = deriveAccent(c, { bg, side, hover, code, hit, soft }, themeInUse(this.value, systemDark).minContrast);
    root.style.setProperty("--accent", d.accent);
    root.style.setProperty("--link", d.accent);
    root.style.setProperty("--accent-text", d.text);
    root.style.setProperty("--accent-soft", d.soft);
  }

  private async applySnippets() {
    document.querySelectorAll("style[data-cairn-snippet]").forEach((el) => el.remove());
    for (const name of this.value.snippets) {
      if (!this.available.includes(name)) continue;
      try {
        const css = await backend.readConfig(`snippets/${name}`);
        if (css == null) continue;
        const el = document.createElement("style");
        el.dataset.cairnSnippet = name;
        el.textContent = css;
        document.head.appendChild(el);
      } catch (e) {
        console.warn("snippet failed", name, e);
      }
    }
  }

  async saveSnippet(name: string, css: string) {
    const file = name.endsWith(".css") ? name : `${name}.css`;
    await backend.writeConfig(`snippets/${file}`, css);
    await this.refreshSnippets();
    await this.applySnippets();
    return file;
  }

  /** Remove theme overrides and the font file when leaving a vault. */
  reset() {
    clearTimeout(this.saveTimer);
    this.dirty = false;
    this.failed = false;
    this.loaded = false;
    this.value = { ...DEFAULT_SETTINGS };
    this.available = [];
    this.dropFont();
    void this.apply();
  }
}

export const settings = new SettingsStore();
