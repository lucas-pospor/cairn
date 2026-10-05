// Vault path helpers, mirroring crates/cairn-core/src/path.rs.

export function parent(p: string): string {
  const i = p.lastIndexOf("/");
  return i < 0 ? "" : p.slice(0, i);
}

export function fileName(p: string): string {
  return p.slice(p.lastIndexOf("/") + 1);
}

export function stem(p: string): string {
  const name = fileName(p);
  const i = name.lastIndexOf(".");
  return i <= 0 ? name : name.slice(0, i);
}

export function extension(p: string): string {
  const name = fileName(p);
  const i = name.lastIndexOf(".");
  return i <= 0 ? "" : name.slice(i + 1).toLowerCase();
}

export function isMarkdown(p: string): boolean {
  const e = extension(p);
  return e === "md" || e === "markdown";
}

/** True if any component starts with "." (dotfiles, `.cairn/`, `.git/`,
 * `.trash/`). Splits on both slashes, so it also works on unnormalized input. */
export function isHidden(p: string): boolean {
  return p.split(/[\\/]/).some((c) => c.startsWith("."));
}

export function join(dir: string, name: string): string {
  return dir ? `${dir}/${name}` : name;
}

export function isInside(p: string, dir: string): boolean {
  if (dir === "") return p !== "";
  return p.length > dir.length && p.startsWith(dir) && p[dir.length] === "/";
}

export function isSameOrInside(p: string, dir: string): boolean {
  return p === dir || isInside(p, dir);
}

export function rebase(p: string, from: string, to: string): string {
  return p === from ? to : join(to, p.slice(from.length + 1));
}

/** Display name: notes without ".md", other files with their extension. */
export function displayName(p: string): string {
  return isMarkdown(p) ? stem(p) : fileName(p);
}

export const FORBIDDEN_CHARS = /[\\/:*?"<>|[\]#^]/;

export const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "bmp", "ico"]);
export const AUDIO_EXTS = new Set(["mp3", "ogg", "oga", "wav", "m4a", "flac"]);
export const VIDEO_EXTS = new Set(["mp4", "m4v", "webm", "mov"]);

export function isImage(p: string): boolean {
  return IMAGE_EXTS.has(extension(p));
}

/** Resolve `rel` (may contain ..) against folder `base`. Null if it escapes. */
export function resolveRelative(base: string, rel: string): string | null {
  const parts = rel.startsWith("/") ? [] : base.split("/").filter(Boolean);
  for (const c of rel.split(/[\\/]/)) {
    if (c === "" || c === ".") continue;
    if (c === "..") {
      if (!parts.length) return null;
      parts.pop();
    } else parts.push(c);
  }
  return parts.join("/");
}
