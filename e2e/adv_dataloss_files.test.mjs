// Adversarial data-loss tests: unusual file contents and file kinds.
// Are the bytes you did not touch preserved when Cairn saves?
//
// Run: scripts/e2e-headless.sh e2e/adv_dataloss_files.test.mjs
// The 50 MB test is slow; enable it with CAIRN_DL_BIG=1.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { Env, eventually, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("files");
});
after(async () => {
  await env?.dispose();
});

async function editAndSave(app, v, rel, insert = "X") {
  await app.openFromTree(rel);
  await app.source();
  await app.insertEnd(insert);
  await app.waitSaved();
  await sleep(200);
}

test("UTF-8 BOM, empty file, NUL and astral characters survive an edit byte for byte", async () => {
  const v = env.vault("v", {
    "bom.md": "﻿# Bom\nbody\n",
    "empty.md": "",
    "odd.md": "nul:\u0000 emoji:\u{1F600} zwj:‍ tab:\t end\n",
  });
  const app = await env.launch(v);
  try {
    await editAndSave(app, v, "bom.md");
    assert.deepEqual(v.bytes("bom.md"), Buffer.from("﻿# Bom\nbody\nX", "utf8"));
    await editAndSave(app, v, "empty.md");
    assert.equal(v.read("empty.md"), "X");
    await editAndSave(app, v, "odd.md");
    assert.deepEqual(v.bytes("odd.md"), Buffer.from("nul:\u0000 emoji:\u{1F600} zwj:‍ tab:\t end\nX", "utf8"));
  } finally {
    await app.close();
  }
});

test("a Latin-1 (non-UTF-8) note is refused, not corrupted: no U+FFFD written back", async () => {
  const v = env.vault("v", { "B.md": "b\n" });
  const latin1 = Buffer.from([0x63, 0x61, 0x66, 0xe9, 0x20, 0x6e, 0x61, 0xef, 0x76, 0x65, 0x0a]); // "café naïve\n"
  fs.writeFileSync(v.p("latin1.md"), latin1);
  const app = await env.launch(v);
  try {
    await app.openFromTree("latin1.md");
    const msg = await app.exec(`return document.querySelector('.pane .empty')?.textContent ?? ''`);
    assert.match(msg, /not valid UTF-8/);
    // Typing now must not write anything to that file.
    await app.exec(`const v = document.querySelector('.cm-editor').__cairnView; v.focus(); v.dispatch({ changes: { from: 0, insert: 'typed' }, userEvent: 'input.type' });`);
    await sleep(1200);
    assert.deepEqual(v.bytes("latin1.md"), latin1);
    // It is still listed (and indexed) even though it cannot be edited.
    assert.ok((await app.invoke("list_entries")).ok.some((e) => e.path === "latin1.md"));
  } finally {
    await app.close();
  }
});

test("a note that turns into Latin-1 on disk while open (clean): nothing is written back over it", async () => {
  const v = env.vault("v", { "n.md": "plain text\n", "B.md": "b\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("n.md");
    await app.source();
    const latin1 = Buffer.from([0x63, 0x61, 0x66, 0xe9, 0x0a]);
    fs.writeFileSync(v.p("n.md"), latin1);
    await sleep(1200);
    await app.insertEnd("MINE");
    await app.waitBanner();
    assert.deepEqual(v.bytes("n.md"), latin1, "conflict instead of overwriting");
  } finally {
    await app.close();
  }
});

test("5 MB note: opens, an edit at the top saves the whole file intact, UI stays responsive", async () => {
  const line = "lorem ipsum dolor sit amet, consectetur adipiscing elit\n";
  const big = line.repeat(90_000); // ~5 MB
  const v = env.vault("v", { "Big.md": big, "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    const t0 = Date.now();
    await app.openFromTree("Big.md");
    const openMs = Date.now() - t0;
    await app.source();
    await app.exec(`window.__gap = { max: 0, last: performance.now() }; setInterval(() => { const n = performance.now(); window.__gap.max = Math.max(window.__gap.max, n - window.__gap.last); window.__gap.last = n; }, 10);`);
    await app.focusAt(0);
    await app.s.keys("hello ");
    await eventually(async () => (await app.saveState()) === "Saved", { timeout: 20000, message: "saved" });
    const gap = await app.exec(`return Math.round(window.__gap.max)`);
    const d = v.bytes("Big.md");
    assert.equal(d.length, Buffer.byteLength(big) + 6);
    assert.equal(d.subarray(0, 6).toString(), "hello ");
    assert.ok(d.subarray(6).equals(Buffer.from(big)), "rest of the file byte-identical");
    assert.ok(openMs < 10000, `open took ${openMs} ms`);
    assert.ok(gap < 1000, `main thread blocked for ${gap} ms`);
  } finally {
    await app.close();
  }
});

test("50 MB note: no hang, bytes preserved (set CAIRN_DL_BIG=1)", { skip: !process.env.CAIRN_DL_BIG && "slow: set CAIRN_DL_BIG=1" }, async () => {
  const line = "lorem ipsum dolor sit amet, consectetur adipiscing elit\n";
  const big = line.repeat(900_000); // ~50 MB
  const v = env.vault("v", { "Big.md": big, "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    const t0 = Date.now();
    await app.openFromTree("Big.md");
    const openMs = Date.now() - t0;
    await app.source();
    await app.exec(`window.__gap = { max: 0, last: performance.now() }; setInterval(() => { const n = performance.now(); window.__gap.max = Math.max(window.__gap.max, n - window.__gap.last); window.__gap.last = n; }, 10);`);
    await app.focusAt(0);
    const t1 = Date.now();
    await app.s.keys("hello");
    const typeMs = Date.now() - t1;
    const t2 = Date.now();
    await eventually(async () => (await app.saveState()) === "Saved", { timeout: 60000, message: "saved" });
    const saveMs = Date.now() - t2;
    const gap = await app.exec(`return Math.round(window.__gap.max)`);
    console.log(`50 MB: open ${openMs} ms, 5 keys ${typeMs} ms, save ${saveMs} ms, longest main-thread stall ${gap} ms`);
    const d = v.bytes("Big.md");
    assert.equal(d.length, Buffer.byteLength(big) + 5);
    assert.ok(d.subarray(5).equals(Buffer.from(big)));
  } finally {
    await app.close();
  }
});

test(
  "CRLF note: lines I did not touch keep their CRLF endings after an edit",
  async () => {
    const v = env.vault("v", { "crlf.md": "line one\r\nline two\r\nline three\r\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await editAndSave(app, v, "crlf.md");
      const disk = v.read("crlf.md");
      assert.ok(disk.startsWith("line one\r\nline two\r\nline three\r\n"), `line endings rewritten: ${JSON.stringify(disk)}`);
    } finally {
      await app.close();
    }
  },
);

test("CRLF note with frontmatter opens with the cursor below it, also when it is only frontmatter", async () => {
  const v = env.vault("v", { "only.md": "---\r\ntags: [a]\r\n---\r\n", "fm.md": "---\r\ntags: [a]\r\n---\r\nbody\r\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  const cursor = () =>
    app.exec(`const st = document.querySelector('.cm-editor').__cairnView.state, l = st.doc.lineAt(st.selection.main.head); return [l.number, st.selection.main.head - l.from]`);
  try {
    await app.openFromTree("only.md");
    assert.equal(await app.editorText(), "---\ntags: [a]\n---\n");
    assert.deepEqual(await cursor(), [4, 0]);
    await app.openFromTree("fm.md");
    assert.deepEqual(await cursor(), [4, 0]);
  } finally {
    await app.close();
  }
});

test(
  "a read-only note (chmod 444) is not silently overwritten",
  async () => {
    const v = env.vault("v", { "ro.md": "do not change\n", "Z.md": "z\n" });
    fs.chmodSync(v.p("ro.md"), 0o444);
    const app = await env.launch(v);
    try {
      await app.openFromTree("ro.md");
      await app.source();
      await app.insertEnd("EDIT");
      await sleep(1500);
      const disk = v.read("ro.md");
      const toasts = await app.toasts();
      assert.ok(
        disk === "do not change\n" || toasts.some((t) => /read-only|permission/i.test(t)),
        `read-only file overwritten silently: disk=${JSON.stringify(disk)} toasts=${JSON.stringify(toasts)} state=${await app.saveState()}`,
      );
    } finally {
      fs.chmodSync(v.p("ro.md"), 0o644);
      await app.close();
    }
  },
);

test(
  "a symlinked note stays a symlink and the edit reaches the real file",
  async () => {
    const v = env.vault("v", { "Z.md": "z\n" });
    const outside = path.join(env.tmp, `outside-${Date.now()}.md`);
    fs.writeFileSync(outside, "real file\n");
    fs.symlinkSync(outside, v.p("link.md"));
    const app = await env.launch(v);
    try {
      await editAndSave(app, v, "link.md", "EDIT");
      const isLink = fs.lstatSync(v.p("link.md")).isSymbolicLink();
      const real = fs.readFileSync(outside, "utf8");
      assert.ok(isLink && real === "real file\nEDIT", `symlink replaced: isLink=${isLink} target=${JSON.stringify(real)} vault copy=${JSON.stringify(v.read("link.md"))}`);
    } finally {
      await app.close();
    }
  },
);

test(
  "a hard-linked note keeps its link (the other name sees the edit)",
  async () => {
    const v = env.vault("v", { "h.md": "shared\n", "Z.md": "z\n" });
    const other = path.join(env.tmp, `hardlink-${Date.now()}.md`);
    fs.linkSync(v.p("h.md"), other);
    const app = await env.launch(v);
    try {
      await editAndSave(app, v, "h.md", "EDIT");
      const o = fs.readFileSync(other, "utf8");
      assert.equal(o, "shared\nEDIT", "hard link broken: the other name still has the old text");
    } finally {
      await app.close();
    }
  },
);

// FINDING-013, config folders: a vault from elsewhere can link .cairn itself
// out of the vault. The settings there are still read, but a change is never
// written there; a toast says why.
test("a .cairn folder that links out of the vault is read but not written", async () => {
  const v = env.vault("v", { "Z.md": "z\n" });
  const outside = path.join(env.tmp, `outside-cairn-${Date.now()}`);
  fs.mkdirSync(outside);
  fs.writeFileSync(path.join(outside, "settings.json"), '{"theme":"dark"}\n');
  fs.symlinkSync(outside, v.p(".cairn"));
  const app = await env.launch(v);
  try {
    await app.s.waitFor(`return document.documentElement.dataset.theme === "dark"`, { message: "the theme from the linked settings.json" });
    await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=settings-appearance]')`);
    await app.exec(`document.querySelector('[data-testid=settings-appearance]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=theme-select]')`);
    await app.exec(`const s = document.querySelector('[data-testid=theme-select]'); s.value = 'light'; s.dispatchEvent(new Event('change', { bubbles: true })); return 1`);
    const refused = 'The ".cairn" folder leads outside the vault.';
    await eventually(async () => (await app.toasts()).includes(`Could not save settings: ${refused}`), { message: "settings toast" });
    // Below the theme rows, the button can be cut off at the bottom of Settings,
    // and WebDriver does not scroll a partly hidden element into view.
    await app.exec(`document.querySelector('[data-testid=snippet-new]').scrollIntoView({ block: 'center' }); return 1`);
    await app.clickTestId("snippet-new");
    await app.clickTestId("snippet-save");
    await eventually(async () => (await app.toasts()).includes(`Could not save the snippet: ${refused}`), { message: "snippet toast" });
    assert.ok(await app.exec(`return !!document.querySelector('[data-testid=snippet-css]')`), "the snippet editor stays open");
    assert.deepEqual(fs.readdirSync(outside), ["settings.json"]);
    assert.equal(fs.readFileSync(path.join(outside, "settings.json"), "utf8"), '{"theme":"dark"}\n');
  } finally {
    await app.close();
  }
});
