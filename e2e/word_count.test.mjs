// The word and character count in the status bar, in the real app: for the note,
// and for the selection while there is one.
//
//   scripts/e2e-headless.sh e2e/word_count.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { CoreApp, eventually } from "./core_lib.mjs";

let app;

before(async () => {
  app = await CoreApp.start("word-count", {
    "Mixed.md": "Hello world, it's a well-known fact.\n我喜欢猫。\n안녕하세요 세계\n",
    "One.md": "Word",
  });
});

after(async () => {
  await app?.stop();
});

const count = () => app.exec(`return document.querySelector('[data-testid=word-count]')?.textContent.trim() ?? null`);
const select = (from, to) => app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: arguments[0], head: arguments[1] } });`, from, to);

test("counts the words and characters of the note, Chinese and Korean too", async () => {
  await app.openNote("Mixed.md");
  // 6 English words, 4 Chinese characters, 2 Korean words; 36 + 5 + 8 characters.
  await eventually(async () => (await count()) === "12 words · 49 characters", { message: "note counts" });
  await app.openNote("One.md");
  await eventually(async () => (await count()) === "1 word · 4 characters", { message: "singular" });
});

test("with a selection, counts the selection, and the note again when it goes", async () => {
  await app.openNote("Mixed.md");
  await eventually(async () => (await count()) === "12 words · 49 characters", { message: "note counts" });
  // "Hello world"
  await select(0, 11);
  await eventually(async () => (await count()) === "2 words · 11 characters selected", { message: "selection counts" });
  // The line of Chinese, with its line break.
  const start = await app.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.line(2).from`);
  await select(start, start + 6);
  await eventually(async () => (await count()) === "4 words · 5 characters selected", { message: "Chinese selection" });
  await select(0, 0);
  await eventually(async () => (await count()) === "12 words · 49 characters", { message: "back to the note" });
});

test("typing updates the count", async () => {
  const end = await app.exec(`return document.querySelector('.cm-editor').__cairnView.state.doc.length`);
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.dispatch({ changes: { from: arguments[0], insert: 'two more' }, userEvent: 'input.type' });`, end);
  await eventually(async () => (await count()) === "14 words · 57 characters", { message: "after typing" });
  await eventually(() => app.read("Mixed.md").endsWith("two more"), { message: "saved" });
  assert.match(await count(), /^14 words/);
});
