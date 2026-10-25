// Properties (frontmatter) in the real app: the properties box in the
// reading view, aliases in the [[ autocomplete and the quick switcher, and a
// template's properties going into the note's.
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
    "Trip.md": '---\ntitle: Trip\ntags: [travel]\nnote: "<img src=x onerror=window.__pwned=1>"\n---\n# Day one\n',
    "Host.md": "# Host\n\n![[Trip]]\n",
  });
});

after(async () => {
  await app?.stop();
});

/** The look of a properties box: what Live Preview and the reading view must share. */
const boxStyle = (css) =>
  app.exec(
    `const b = document.querySelector(arguments[0]); if (!b) return null; const st = getComputedStyle(b);
     return { border: st.borderTopWidth + " " + st.borderTopStyle + " " + st.borderTopColor, radius: st.borderTopLeftRadius, background: st.backgroundColor, font: st.fontSize };`,
    css,
  );

test("the reading view shows the properties as the box Live Preview shows", async () => {
  await app.openNote("Trip.md");
  await app.click("[data-testid=mode-live]");
  await app.waitFor(`return !!document.querySelector('.cm-lp-props .props')`, { message: "Live Preview box" });
  const live = await boxStyle(".cm-lp-props .props");
  await app.click("[data-testid=mode-preview]");
  await app.waitFor(`return !!document.querySelector('[data-testid=preview] .md-render .md-props .props')`, { message: "reading view box" });
  const read = await app.exec(`const r = document.querySelector('[data-testid=preview] .md-render');
    return { keys: [...r.querySelectorAll('.md-props .prop-key')].map((e) => e.textContent), tags: [...r.querySelectorAll('.md-props .tag')].map((e) => e.textContent),
      first: r.firstElementChild?.className, heading: r.querySelector('h1')?.textContent, text: r.textContent, imgs: r.querySelectorAll('img').length };`);
  assert.deepEqual(read.keys, ["title", "tags", "note"]);
  assert.deepEqual(read.tags, ["#travel"]);
  assert.equal(read.first, "md-props");
  assert.equal(read.heading, "Day one");
  // A hostile value shows as text and runs nothing.
  assert.ok(read.text.includes("<img src=x onerror=window.__pwned=1>"), read.text);
  assert.equal(read.imgs, 0);
  assert.equal(await app.exec(`return window.__pwned ?? null`), null);
  assert.deepEqual(await boxStyle("[data-testid=preview] .md-props .props"), live);
});

test("an embedded note shows its body without its properties", async () => {
  await app.openNote("Host.md");
  await app.click("[data-testid=mode-preview]");
  await app.waitFor(`return !!document.querySelector('[data-testid=preview] .md-render .embed-body h1')`, { message: "embed filled" });
  assert.equal(await app.exec(`return document.querySelectorAll('[data-testid=preview] .md-props').length`), 0);
  await app.click("[data-testid=mode-live]");
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
