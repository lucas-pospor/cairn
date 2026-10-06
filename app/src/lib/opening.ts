// Where a vault file opens when the user opens it (file tree, quick switcher,
// search, a link): notes and images in a tab of their own, other files in the
// system's default app on the desktop. Android cannot hand files to other
// apps yet.

import { isImage, isMarkdown } from "./paths";

export type OpenTarget = "note" | "image" | "default-app" | "unavailable";

export function openTarget(path: string, mobile: boolean): OpenTarget {
  if (isMarkdown(path)) return "note";
  if (isImage(path)) return "image";
  return mobile ? "unavailable" : "default-app";
}

/**
 * The vault path of the embedded image an event is on (the renderer marks
 * images of the vault with data-path), or null. An image inside a link
 * belongs to the link: a click there follows the link, as before.
 */
export function embeddedImageAt(target: EventTarget | null): string | null {
  if (!(target instanceof Element)) return null;
  const img = target.closest("img[data-path]");
  return img instanceof HTMLElement && !img.closest("a") ? (img.dataset.path ?? null) : null;
}
