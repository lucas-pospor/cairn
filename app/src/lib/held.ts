// What the page tells the backend about edits that are not on disk yet, on
// Windows, so that the backend can save them, or name them, when Windows
// ends the session and the page cannot answer (app/src-tauri/src/held.rs
// and session_end.rs). The page sends only what changed since the backend
// last took a request, each entry whole.

/** What keeps a note from being saved. */
export type Problem = "conflict" | "failed" | null;

/** A note tab with unsaved edits, as session_hold takes it. */
export interface HeldNote {
  path: string;
  /** The hash of the file the edits are based on. */
  base: string | null;
  /** The tab's edit number. */
  edit: number;
  problem: Problem;
  /**
   * The note's text as a save writes it; null when the page does not send
   * it (too large, or not well formed). Left out: the text the backend has
   * for the same edit number. Never sent with a problem.
   */
  text?: string | null;
}

export interface HeldRelease {
  path: string;
  release: true;
}

/** What the page wants the backend to hold for a path. */
export interface Wanted {
  base: string | null;
  edit: number;
  problem: Problem;
}

/** What the backend took last for a path. */
export interface Sent extends Wanted {
  /** Whether it has the text for `edit`. */
  text: boolean;
}

/** The fields of a tab the backend's copy follows. */
export interface HeldTab {
  kind: string;
  path: string;
  dirty: boolean;
  conflict: string | null;
  saveFailed: boolean;
  baseHash: string | null;
  edit: number;
  /** The tab's text can be read (it loaded). */
  loaded: boolean;
  /** The user discarded its edits; it is about to go. */
  discarded: boolean;
}

/**
 * Notes over this many UTF-16 units (the editor's length plus a line break
 * each, for CRLF files) are not sent while the user types: only when the
 * backend asks at the end of the session.
 */
export const TEXT_LIMIT = 1_000_000;

/** What the backend should hold: every note tab with unsaved edits, by path. */
export function wanted(tabs: readonly HeldTab[]): Map<string, Wanted> {
  const out = new Map<string, Wanted>();
  for (const t of tabs) {
    if (t.kind !== "note" || !t.dirty || t.discarded || !t.loaded) continue;
    const problem: Problem = t.conflict !== null ? "conflict" : t.saveFailed ? "failed" : null;
    const other = out.get(t.path);
    if (other) {
      // Two tabs with edits for one file (one renamed onto the other): which
      // one to keep is the user's choice.
      out.set(t.path, { base: other.base, edit: Math.max(other.edit, t.edit), problem: "conflict" });
    } else {
      out.set(t.path, { base: t.baseHash, edit: t.edit, problem });
    }
  }
  return out;
}

/**
 * The entries to send so that the backend, which took `sent` last, holds
 * `want`, and what it then holds. `text` gives a note's text, or null when
 * it is not to be sent; it is asked only for a note whose text the backend
 * does not have yet.
 */
export function changes(
  sent: ReadonlyMap<string, Sent>,
  want: ReadonlyMap<string, Wanted>,
  text: (path: string) => string | null,
): { notes: (HeldNote | HeldRelease)[]; next: Map<string, Sent> } {
  const notes: (HeldNote | HeldRelease)[] = [];
  const next = new Map<string, Sent>();
  for (const [path, w] of want) {
    const s = sent.get(path);
    if (s && s.base === w.base && s.edit === w.edit && s.problem === w.problem) {
      next.set(path, s);
      continue;
    }
    const note: HeldNote = { path, ...w };
    let withText = false;
    if (w.problem === null) {
      // The backend keeps its text for the same edit when none is sent.
      if (s?.edit === w.edit && s.text) withText = true;
      else {
        note.text = text(path);
        withText = note.text !== null;
      }
    }
    notes.push(note);
    next.set(path, { ...w, text: withText });
  }
  for (const path of sent.keys()) if (!want.has(path)) notes.push({ path, release: true });
  return { notes, next };
}

/** A string with no lone surrogate, which JSON could not carry to the backend. */
export function wellFormed(s: string): boolean {
  return !/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/.test(s);
}
