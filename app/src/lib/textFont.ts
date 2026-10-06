// A font file of the user's for note text (Settings, then Appearance, then
// Font file). The file is kept in the vault's `.cairn/fonts/` and named by the
// "textFont" setting, so it travels with the folder; Cairn sync does not copy
// `.cairn/`. It is loaded with the CSS Font Loading API from its bytes: the
// web view's CSP allows fonts only from the app and data: URLs, and vault://
// does not serve dot-folders. Wherever it cannot load, the Text font is used.

import { errorMessage, isCoreError } from "./types";

/** The folder in `.cairn/` that holds the font. */
export const FONT_DIR = "fonts";
export const FONT_EXTENSIONS = ["woff2", "woff", "ttf", "otf"] as const;
/** 20 MB covers most fonts with Chinese, Japanese or Korean characters. */
export const MAX_FONT_BYTES = 20 * 1024 * 1024;

/** Why `name` cannot be the file name of a font in `.cairn/fonts/`, or null. */
export function fontNameProblem(name: unknown): string | null {
  if (typeof name !== "string" || !name.trim()) return "The font has no file name.";
  if (/[\\/\0]/.test(name) || name.startsWith(".")) return "A font file name cannot contain slashes or start with a dot.";
  const ext = name.slice(name.lastIndexOf(".") + 1).toLowerCase();
  if (!name.includes(".") || !(FONT_EXTENSIONS as readonly string[]).includes(ext)) return "Cairn takes woff2, woff, ttf and otf font files.";
  return null;
}

/** The format that the first bytes of a font file show, or null for anything else. */
export function fontFormat(bytes: Uint8Array): "woff2" | "woff" | "truetype" | "opentype" | "collection" | null {
  if (bytes.length < 12) return null;
  const tag = String.fromCharCode(...bytes.subarray(0, 4));
  if (tag === "wOF2") return "woff2";
  if (tag === "wOFF") return "woff";
  if (tag === "OTTO") return "opentype";
  if (tag === "true" || (bytes[0] === 0 && bytes[1] === 1 && bytes[2] === 0 && bytes[3] === 0)) return "truetype";
  if (tag === "ttcf") return "collection";
  return null;
}

/** Why a font file of `size` bytes is too large, or null. */
export function fontSizeProblem(size: number): string | null {
  return size > MAX_FONT_BYTES ? `The font file is ${Math.ceil(size / 2 ** 20)} MB; Cairn takes font files up to 20 MB.` : null;
}

/** Why `bytes` are not a font file Cairn takes, or null. */
export function fontBytesProblem(bytes: Uint8Array): string | null {
  const big = fontSizeProblem(bytes.length);
  if (big) return big;
  const format = fontFormat(bytes);
  if (format === "collection") return "Font collections (ttc) are not supported. Choose a woff2, woff, ttf or otf file.";
  if (!format) return "This is not a woff2, woff, ttf or otf font file.";
  return null;
}

/** Why a font file could not be read or loaded, for a message: an error's own text, without its name. */
export function fontFailure(e: unknown): string {
  if (isCoreError(e) && e.kind === "notFound") return `It is not in .cairn/${FONT_DIR}/.`;
  return e instanceof Error ? e.message : errorMessage(e);
}

let families = 0;

/** A font made from `bytes` and loaded, under a family name of its own; rejects if the web view cannot use it. */
export async function loadFontFace(bytes: ArrayBuffer): Promise<FontFace> {
  if (typeof FontFace !== "function") throw new Error("This web view cannot load fonts.");
  try {
    // WebKit parses the bytes in the constructor and may reject loaded at once.
    const face = new FontFace(`cairn-text-font-${++families}`, bytes);
    await face.load();
    return face;
  } catch {
    // WebKit rejects bad font data as a network error, which would mislead.
    throw new Error("The web view cannot read the font in this file.");
  }
}
