// Platform checks for layout and features.

const ua = typeof navigator !== "undefined" ? navigator.userAgent : "";

export const isAndroid = /Android/i.test(ua);
export const isMobile = isAndroid || /iPhone|iPad|iPod/i.test(ua);

/** Narrow layout: sidebars become drawers. */
export function narrowQuery(): MediaQueryList | null {
  return typeof matchMedia === "function" ? matchMedia("(max-width: 760px)") : null;
}

/** Human-friendly name for a vault location (folder path or content:// URI). */
export function vaultLabel(root: string): string {
  if (root.startsWith("content://")) {
    const decoded = decodeURIComponent(root);
    return decoded.split(/[/:]/).filter(Boolean).pop() ?? "Notebook";
  }
  return root.split(/[\\/]/).filter(Boolean).pop() ?? root;
}
