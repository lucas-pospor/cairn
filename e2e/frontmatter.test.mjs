// Properties (frontmatter) in the real app: aliases in the [[ autocomplete
// and the quick switcher, and a template's properties going into the note's.
//
//   scripts/e2e-headless.sh e2e/frontmatter.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { CoreApp, Key, eventually, sleep } from "./core_lib.mjs";

let app;

before(async () => {
  app = await CoreApp.start("frontmatter", {
    "Start.md": "# Start\n",
    "People/Bob.md": "---\naliases: [Robert, Bobby]\n---\n# Bob\n",
    "Notes/Standup.md": "---\ntitle: Mine\n---\nIntro\n",
    "Templates/Meeting.md": "---\ntitle: Theirs\ntags: [meeting]\n---\n## Agenda\n",
  });
});

after(async () => {
  await app?.stop();
});

const options = () => app.exec(`return [...document.querySelectorAll('.cm-tooltip-autocomplete li')].map(li => li.textContent)`);

async function cursorAtEnd() {
  await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ selection: { anchor: v.state.doc.length } });`);
}

test("the [[ autocomplete offers a note's aliases and links with the alias as text", async () => {
  await app.openNote("Start.md");
  await cursorAtEnd();
  await app.s.keys(Key.enter, "[[Rober");
  await app.waitFor(`return !!document.querySelector('.cm-tooltip-autocomplete li')`, { message: "completion list" });
  const opts = await options();
  assert.equal(opts[0], "Robert↪ Bob", JSON.stringify(opts));
  await sleep(150); // CodeMirror ignores Enter right after the popup opens
  await app.s.keys(Key.enter);
  await eventually(async () => (await app.doc()).endsWith("[[Bob|Robert]]"), { message: "alias link inserted" });
  // It resolves to the note: the alias shows, not an unresolved link.
  const marks = await app.exec(`return [...document.querySelectorAll('.cm-wikilink')].map(e => e.className + ' ' + e.textContent)`);
  assert.ok(marks.some((m) => m.includes("Robert") && !m.includes("unresolved")), JSON.stringify(marks));
  await eventually(() => app.read("Start.md").endsWith("[[Bob|Robert]]"), { message: "saved" });
});

test("aliases changed outside Cairn are offered after the change", async () => {
  app.write("People/Bob.md", "---\naliases: [Roberto]\n---\n# Bob\n");
  await eventually(
    async () => {
      await cursorAtEnd();
      await app.s.keys(Key.enter, "[[Robert");
      await app.waitFor(`return !!document.querySelector('.cm-tooltip-autocomplete li')`);
      const opts = await options();
      await app.s.keys(Key.escape);
      return opts.includes("Roberto↪ Bob") && !opts.some((o) => o.startsWith("Robert↪"));
    },
    { timeout: 10000, message: "new alias offered, old one gone" },
  );
});

test("the quick switcher finds a note by its alias and opens it", async () => {
  await app.s.keys({ chord: [Key.ctrl, "o"] });
  await app.waitFor(`return document.activeElement?.dataset.testid === 'switcher-input'`);
  await app.s.keys("Roberto");
  await app.waitFor(`return document.querySelector('[data-testid=switcher-item]')?.textContent.includes('Roberto')`, { message: "alias row" });
  assert.equal(await app.exec(`return document.querySelector('[data-testid=switcher-item] [data-testid=switcher-alias-of]')?.textContent`), "↪ Bob");
  // An alias is not a note of that name: creating one is still offered.
  assert.ok(await app.exec(`return !!document.querySelector('[data-testid=switcher-create]')`));
  await app.s.keys(Key.enter);
  await eventually(async () => (await app.activeTab()) === "People/Bob.md", { message: "note opened" });
});

test("a template's properties go into the note's, and one undo takes the whole template out", async () => {
  await app.openNote("Notes/Standup.md");
  await cursorAtEnd();
  await app.runCommand("Templates: Insert template");
  await app.waitFor(`return [...document.querySelectorAll('.choice')].some((b) => b.textContent === "Meeting")`);
  await app.exec(`[...document.querySelectorAll('.choice')].find((b) => b.textContent === "Meeting").click()`);
  const want = "---\ntitle: Mine\ntags: [meeting]\n---\nIntro\n## Agenda\n";
  await eventually(async () => (await app.doc()) === want, { message: "template merged" });
  await eventually(() => app.read("Notes/Standup.md") === want, { message: "saved" });
  // The new tag counts at once.
  await app.exec(`document.querySelector('[data-testid=tab-tags]').click()`);
  await app.waitFor(`return document.querySelector('[data-testid=tags-list]')?.textContent.includes('meeting')`, { message: "tag listed" });
  await app.exec(`document.querySelector('.cm-editor').__cairnView.focus()`);
  await app.s.keys({ chord: [Key.ctrl, "z"] });
  const before = "---\ntitle: Mine\n---\nIntro\n";
  await eventually(async () => (await app.doc()) === before, { message: "one undo" });
  await eventually(() => app.read("Notes/Standup.md") === before, { message: "undo saved" });
});
