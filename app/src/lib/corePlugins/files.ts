// Folders and notes for the core plugins.

import { isHidden, isInside, isMarkdown, join } from "../paths";
import { errorMessage, isCoreError } from "../types";
import type { CoreHost } from "./core";

/** A folder option as a vault path: no slashes at either end, "" for the vault root. */
export function cleanFolder(folder: string): string {
  return folder
    .trim()
    .normalize("NFC")
    .split(/[\\/]+/)
    .filter(Boolean)
    .join("/");
}

/** The notes in `folder` and its subfolders, sorted by path. */
export function notesIn(folder: string, files: string[]): string[] {
  return files.filter((p) => isMarkdown(p) && !isHidden(p) && (folder === "" || isInside(p, folder))).sort((a, b) => a.localeCompare(b));
}

/** The path of a note named `name` (which may hold slashes for subfolders) in `folder`. */
export function notePath(folder: string, name: string): string {
  const path = join(cleanFolder(folder), name.normalize("NFC"));
  return isMarkdown(path) ? path : `${path}.md`;
}

/** A path to a note: the option's text, with ".md" added when it has no Markdown extension. */
export function noteOption(value: string): string {
  const path = cleanFolder(value);
  return !path || isMarkdown(path) ? path : `${path}.md`;
}

// What cairn-core (path.rs, validate_name) refuses in the name of a new file or folder.
const FORBIDDEN = /[\\:*?"<>|[\]#^]/;
const NAME_MAX = 255;
// Names Windows keeps for devices, with any extension and spaces before it
// (path.rs, is_reserved_name). Refused for the note's own name: a folder that
// is there keeps its name.
const RESERVED = /^(con|prn|aux|nul|conin\$|conout\$|(com|lpt)[0-9¹²³]) *$/i;

/** Why a new note cannot be created at `path`, as the core sees it, or null when it can. */
export function notePathProblem(path: string): string | null {
  const parts = path.split("/");
  for (const [i, part] of parts.entries()) {
    if (!part) return "A folder name in it is empty.";
    if (part.trim() !== part) return `"${part}" starts or ends with a space.`;
    if (part.startsWith(".")) return `"${part}" starts with a dot, which would hide it.`;
    const bad = FORBIDDEN.exec(part)?.[0];
    if (bad) return `"${part}" contains ${bad}, which names cannot contain.`;
    if (/\p{Cc}/u.test(part)) return `"${part}" contains a control character.`;
    if (part.endsWith(".")) return `"${part}" ends with a dot.`;
    if (i === parts.length - 1 && RESERVED.test(part.split(".")[0])) return `"${part}" is a name Windows keeps for a device.`;
    if (new TextEncoder().encode(part).length > NAME_MAX) return `"${part.slice(0, 20)}…" is too long for a file name.`;
  }
  return null;
}

/** The Settings check of a folder that new notes go into: why it cannot hold them, if it cannot. */
export function folderCheck(value: string): { problem?: string } {
  const folder = cleanFolder(value);
  const problem = folder && notePathProblem(folder);
  return problem ? { problem } : {};
}

/** The Settings check of a template option: the note must be there. */
export function templateCheck(value: string, host: CoreHost): { problem?: string } {
  const template = noteOption(value);
  return !template || host.files().includes(template) ? {} : { problem: `There is no note at ${template}.` };
}

/**
 * Create the note at `path` with `content`, and open it. Nothing is ever written
 * over: when a note is already there (sync or another app made it a moment ago,
 * or its name differs only in case), that note opens instead.
 */
export async function createOrOpen(host: CoreHost, path: string, content: string, newTab = false) {
  try {
    await host.createNote(path, content);
  } catch (e) {
    const there = isCoreError(e) && e.kind === "alreadyExists" ? e.detail || path : null;
    if (there && isMarkdown(there)) return host.openNote(there, newTab);
    return host.toast(`Could not create ${path}: ${errorMessage(e)}`, "error");
  }
  await host.openNote(path, newTab);
}
