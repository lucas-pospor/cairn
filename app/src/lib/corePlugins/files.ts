// Folders and notes for the core plugins' options.

import { isHidden, isInside, isMarkdown } from "../paths";

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
