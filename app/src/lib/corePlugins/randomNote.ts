// Random note: open a note picked at random, other than the one already open
// when there is another.

import type { CoreHost, CorePlugin } from "./core";
import { notesIn } from "./files";

async function openRandom(host: CoreHost) {
  const notes = notesIn("", host.files());
  if (!notes.length) return host.toast("There are no notes in this notebook.");
  const others = notes.filter((p) => p !== host.activeNote());
  const pool = others.length ? others : notes;
  await host.openNote(pool[Math.min(pool.length - 1, Math.floor(host.random() * pool.length))]);
}

export const randomNote: CorePlugin = {
  id: "random-note",
  name: "Random note",
  description: "Open a note picked at random.",
  defaultOn: false,
  defaults: {},
  options: [],
  commands: [{ id: "open", name: "Open random note", run: openRandom }],
};
