import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { keymap, type KeyBinding } from "@codemirror/view";
import { redo } from "@codemirror/commands";
import { linkAtCursor, noteExtensions, type EditorHooks } from "./setup";

const at = (doc: string, cursor: number) => linkAtCursor(EditorState.create({ doc, selection: { anchor: cursor } }));

describe("linkAtCursor (Follow link under cursor, FINDING-124)", () => {
  const doc = "Go to [[Ideas#Plan]] or [site](https://example.com) now.\n[[Next]]";

  it("finds the wikilink or URL the cursor is in", () => {
    expect(at(doc, doc.indexOf("Ideas") + 2)).toEqual({ kind: "wiki", target: "Ideas", subpath: "Plan" });
    expect(at(doc, doc.indexOf("site"))).toEqual({ kind: "url", url: "https://example.com" });
  });

  it("counts the cursor just after a link, as after typing its ]]", () => {
    expect(at(doc, doc.indexOf("]]") + 2)).toEqual({ kind: "wiki", target: "Ideas", subpath: "Plan" });
  });

  it("finds nothing away from links or on the next line", () => {
    expect(at(doc, 1)).toBeNull();
    expect(at(doc, doc.indexOf("[["))).toBeNull();
    // The start of a line does not look back at the end of the previous one.
    expect(at("[[A]]\nplain", "[[A]]\n".length)).toBeNull();
  });
});

describe("redo keys in the editor", () => {
  // The key each binding of the note editor gives Redo on a platform, as
  // CodeMirror picks it: the platform's own name, else the plain key.
  const redoKeys = (platform: "win" | "linux" | "mac") => {
    const state = EditorState.create({ extensions: noteExtensions({} as EditorHooks) });
    const bindings: KeyBinding[] = state.facet(keymap).flat();
    return bindings.filter((b) => b.run === redo).map((b) => b[platform] ?? b.key).filter(Boolean);
  };

  it("redoes on Ctrl+Shift+Z on Windows as on Linux, and keeps Ctrl+Y", () => {
    expect(redoKeys("win")).toEqual(["Mod-y", "Mod-Shift-z"]);
    expect(redoKeys("linux")).toEqual(["Mod-y", "Ctrl-Shift-z"]);
    expect(redoKeys("mac")).toEqual(["Mod-Shift-z"]);
  });
});
