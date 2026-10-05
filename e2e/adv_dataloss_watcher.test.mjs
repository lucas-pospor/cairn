// Adversarial tests for the desktop file watcher: other editors' save
// strategies, folder operations, bulk changes, git checkout, event floods.
//
// Run: scripts/e2e-headless.sh e2e/adv_dataloss_watcher.test.mjs
// The test marked { todo: "not a defect (by design): ..." } keeps documenting
// behaviour that is by design: it prints its failure but does not fail the run.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { execFileSync } from "node:child_process";
import { Env, eventually, sleep, Key } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("watch");
});
after(async () => {
  await env?.dispose();
});

const saveStyles = {
  // gedit / JetBrains "safe write" / many editors: temp file then rename over
  "temp file + rename (hidden temp)": (v, rel, c) => {
    v.write(`.${rel}.tmp-save`, c);
    v.mv(`.${rel}.tmp-save`, rel);
  },
  "temp file + rename (visible temp)": (v, rel, c) => {
    v.write(`${rel}.tmp123`, c);
    v.mv(`${rel}.tmp123`, rel);
  },
  // vim with backupcopy=no: original renamed to a backup, new file written
  "vim backup rename": (v, rel, c) => {
    v.mv(rel, `${rel}~`);
    v.write(rel, c);
    v.rm(`${rel}~`);
  },
  // git checkout, some sync clients
  "delete + recreate": (v, rel, c) => {
    v.rm(rel);
    v.write(rel, c);
  },
};

for (const [name, save] of Object.entries(saveStyles)) {
  test(`other editor saves by "${name}": clean tab reloads, dirty tab gets the banner, disk keeps theirs`, async () => {
    const v = env.vault("v", { "Clean.md": "clean original\n", "Dirty.md": "dirty original\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("Clean.md");
      await app.openFromTree("Dirty.md", { newTab: true });
      await app.source();
      await sleep(300);
      save(v, "Clean.md", "clean EXTERNAL\n");
      await sleep(900);
      await app.insertEnd("MINE");
      save(v, "Dirty.md", "dirty EXTERNAL\n");
      const banner = await app.waitBanner();
      assert.match(banner, /changed on disk/);
      assert.equal(await app.editorText(), "dirty original\nMINE");
      await sleep(700);
      assert.equal(v.read("Dirty.md"), "dirty EXTERNAL\n");
      const tabs = await app.tabs();
      assert.deepEqual(tabs.map((t) => t.path), ["Clean.md", "Dirty.md"]);
      await app.exec(`[...document.querySelectorAll('[data-testid=tab]')].find(t => t.dataset.path === 'Clean.md').click()`);
      await eventually(async () => (await app.editorText()) === "clean EXTERNAL\n", { message: "clean tab reloaded" });
    } finally {
      await app.close();
    }
  });
}

test("external folder delete with dirty and clean tabs inside: clean tab closes, dirty tab keeps edits with banner", async () => {
  const v = env.vault("v", { "d/Clean.md": "c\n", "d/Dirty.md": "d\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("d/Clean.md");
    await app.openFromTree("d/Dirty.md", { newTab: true });
    await app.source();
    await app.insertEnd("MINE");
    v.rm("d");
    await app.waitBanner();
    const tabs = await app.tabs();
    assert.deepEqual(tabs.map((t) => t.path), ["d/Dirty.md"]);
    assert.equal(await app.editorText(), "d\nMINE");
    await app.clickButtonText("Save my version");
    await eventually(() => v.exists("d/Dirty.md") && v.read("d/Dirty.md") === "d\nMINE", { message: "recreated with parent folder" });
  } finally {
    await app.close();
  }
});

test("5,000 files copied into the vault while a note is open: index matches disk, switcher finds them, no freeze, open tab untouched", async () => {
  const v = env.vault("v", { "Open.md": "open note\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("Open.md");
    await app.source();
    await app.insertEnd("MINE");
    // Let the autosave finish first: a save that lands within 60 ms of the
    // bulk "created" event cancels the tree refresh (FINDING-048).
    await eventually(() => v.read("Open.md") === "open note\nMINE", { message: "autosaved" });
    await app.waitSaved();
    await sleep(500);
    await app.exec(`window.__gap = { max: 0, last: performance.now() }; setInterval(() => { const n = performance.now(); window.__gap.max = Math.max(window.__gap.max, n - window.__gap.last); window.__gap.last = n; }, 10);`);
    fs.mkdirSync(v.p("import/sub"), { recursive: true });
    for (let i = 0; i < 5000; i++) {
      const dir = i % 2 ? "import" : "import/sub";
      fs.writeFileSync(v.p(`${dir}/note ${String(i).padStart(4, "0")}.md`), `imported ${i} [[Open]]\n`);
    }
    const disk = v.listDisk();
    await eventually(async () => (await app.invoke("list_entries")).ok.length === disk.length, { timeout: 15000, message: "index caught up" });
    const idx = (await app.invoke("list_entries")).ok.map((e) => e.path).sort();
    assert.deepEqual(idx, disk);
    const gap = await app.exec(`return Math.round(window.__gap.max)`);
    assert.ok(gap < 1000, `UI blocked for ${gap} ms`);
    assert.equal(v.read("Open.md"), "open note\nMINE", "open note untouched by the bulk copy");
    assert.equal(await app.banner(), null, "no banner");
    // The UI's own file list (quick switcher) has them too.
    await sleep(500);
    await app.s.keys({ chord: [Key.ctrl, "o"] });
    await app.s.type(await app.s.findWait("[data-testid=switcher-input]"), "note 4999");
    await app.s.waitFor(`return [...document.querySelectorAll('[data-testid=switcher-item]')].some(e => e.textContent.includes('note 4999'))`, { timeout: 5000 }).catch(async (e) => {
      const items = await app.exec(`return [...document.querySelectorAll('[data-testid=switcher-item]')].slice(0, 5).map(e => e.textContent)`);
      const q = await app.exec(`return document.querySelector('[data-testid=switcher-input]')?.value`);
      throw new Error(`${e.message}; query=${JSON.stringify(q)} items=${JSON.stringify(items)}`);
    });
    await app.s.keys(Key.escape);
  } finally {
    await app.close();
  }
});

test("git checkout of another branch with clean, dirty and soon-deleted tabs open", async () => {
  const v = env.vault("v", { "Clean.md": "clean main\n", "Dirty.md": "dirty main\n", "Gone.md": "only on main\n" });
  const gitEnv = { ...process.env, GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@t", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@t", GIT_CONFIG_GLOBAL: "/dev/null" };
  const git = (...a) => execFileSync("git", a, { cwd: v.root, encoding: "utf8", env: gitEnv });
  git("init", "-q", "-b", "main");
  git("add", "-A");
  git("commit", "-qm", "main");
  git("checkout", "-qb", "other");
  v.write("Clean.md", "clean OTHER BRANCH\n");
  v.write("Dirty.md", "dirty OTHER BRANCH\n");
  v.rm("Gone.md");
  for (let i = 0; i < 2000; i++) v.write(`many/n${i}.md`, `n${i}\n`);
  git("add", "-A");
  git("commit", "-qm", "other");
  git("checkout", "-q", "main");
  const app = await env.launch(v);
  try {
    await app.openFromTree("Clean.md");
    await app.openFromTree("Gone.md", { newTab: true });
    await app.openFromTree("Dirty.md", { newTab: true });
    await app.source();
    await app.insertEnd("MINE");
    git("checkout", "-q", "other");
    await app.waitBanner();
    await eventually(async () => (await app.invoke("list_entries")).ok.length === v.listDisk().length, { timeout: 10000, message: "index caught up" });
    assert.deepEqual((await app.invoke("list_entries")).ok.map((e) => e.path).sort(), v.listDisk());
    assert.equal(v.read("Dirty.md"), "dirty OTHER BRANCH\n", "dirty tab did not clobber the checkout");
    assert.equal(await app.editorText(), "dirty main\nMINE", "dirty tab kept my edits");
    const tabs = (await app.tabs()).map((t) => t.path);
    assert.deepEqual(tabs, ["Clean.md", "Dirty.md"], "deleted note's clean tab closed");
    await app.exec(`[...document.querySelectorAll('[data-testid=tab]')].find(t => t.dataset.path === 'Clean.md').click()`);
    await eventually(async () => (await app.editorText()) === "clean OTHER BRANCH\n", { message: "clean tab reloaded" });
  } finally {
    await app.close();
  }
});

test(
  "folder renamed right after something read a file in it: open tab follows the rename",
  async () => {
    const v = env.vault("v", { "dir/C.md": "charlie\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("dir/C.md");
      await sleep(800);
      fs.readFileSync(v.p("dir/C.md")); // any reader: backup tool, indexer, grep, Cairn itself
      await sleep(100);
      v.mv("dir", "dir2");
      await sleep(1500);
      await app.shot("DL-09-after-folder-rename");
      const tabs = await app.tabs();
      assert.deepEqual(tabs.map((t) => t.path), ["dir2/C.md"], `tab did not follow the folder rename: ${JSON.stringify(tabs)}`);
    } finally {
      await app.close();
    }
  },
);

test(
  "folder renamed right after Cairn saved a note in it: unsaved edits follow, no false 'deleted' banner",
  async () => {
    const v = env.vault("v", { "dir/C.md": "charlie\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("dir/C.md");
      await app.source();
      await app.insertEnd("saved ");
      await eventually(() => v.read("dir/C.md") === "charlie\nsaved ", { message: "autosaved" });
      await sleep(100); // the watcher has not digested Cairn's own write yet
      await app.insertEnd("UNSAVED");
      v.mv("dir", "dir2");
      await sleep(2000);
      const banner = await app.banner();
      assert.equal(banner, null, `false banner: ${banner}`);
      assert.equal(v.read("dir2/C.md"), "charlie\nsaved UNSAVED");
      assert.ok(!v.exists("dir"));
    } finally {
      await app.close();
    }
  },
);

test(
  "a slow replace (delete, then recreate 0.6 s later) keeps the clean tab open",
  { todo: "not a defect (by design): a slow delete+recreate or slow vim-style save closes the open tab" },
  async () => {
    const v = env.vault("v", { "N.md": "original\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("N.md");
      await sleep(300);
      v.rm("N.md");
      await sleep(600);
      v.write("N.md", "replaced\n");
      await sleep(1500);
      const tabs = await app.tabs();
      assert.ok(tabs.some((t) => t.path === "N.md"), `tab was closed although the file is back: ${JSON.stringify(tabs)}`);
    } finally {
      await app.close();
    }
  },
);

test(
  "files created during an inotify queue overflow still show up",
  async () => {
    const v = env.vault("v", { "Open.md": "open\n", "log-a.md": "", "log-b.md": "" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("Open.md");
      await sleep(500);
      // A burst of events on two files (a log being appended, a build, a
      // big git operation inside .git) overflows the kernel queue
      // (fs.inotify.max_queued_events, 16384 by default).
      const fa = fs.openSync(v.p("log-a.md"), "a");
      const fb = fs.openSync(v.p("log-b.md"), "a");
      for (let i = 0; i < 600_000; i++) {
        fs.writeSync(i % 2 ? fa : fb, "x");
        if (i > 30_000 && i % 5_000 === 0) fs.writeFileSync(v.p(`created-during-burst-${i}.md`), `made at ${i}\n`);
      }
      fs.closeSync(fa);
      fs.closeSync(fb);
      await sleep(3000);
      const idx = (await app.invoke("list_entries")).ok.map((e) => e.path).sort();
      const disk = v.listDisk();
      const missing = disk.filter((p) => !idx.includes(p));
      assert.deepEqual(missing, [], `${missing.length} of ${disk.length} files never indexed`);
    } finally {
      await app.close();
    }
  },
);

test(
  "a file created outside shows up in the tree even if I save another note right then",
  async () => {
    const v = env.vault("v", { "Open.md": "open\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("Open.md");
      await app.source();
      // When the watcher reports the new file, save the open note at once
      // (Ctrl+S). A real user hits this with plain autosave timing, about
      // 1 time in 20 in testing; the hook makes it deterministic.
      await app.exec(`
        const I = window.__TAURI_INTERNALS__;
        window.__armed = true;
        I.invoke('plugin:event|listen', { event: 'vault-changed', target: { kind: 'Any' }, handler: I.transformCallback((e) => {
          if (!window.__armed || !e.payload.some((c) => c.type === 'created')) return;
          window.__armed = false;
          const v = document.querySelector('.cm-editor').__cairnView;
          v.dispatch({ changes: { from: v.state.doc.length, insert: '!' }, userEvent: 'input.type' });
          window.dispatchEvent(new KeyboardEvent('keydown', { key: 's', code: 'KeyS', ctrlKey: true, bubbles: true, cancelable: true }));
        }) });
      `);
      await sleep(200);
      v.write("Brand new.md", "made outside\n");
      await eventually(() => v.read("Open.md") === "open\n!", { message: "Ctrl+S saved the open note" });
      await sleep(1500);
      const inIndex = (await app.invoke("list_entries")).ok.some((e) => e.path === "Brand new.md");
      const inTree = await app.exec(`return !!document.querySelector('[data-testid=tree-row][data-path="Brand new.md"]')`);
      assert.ok(inIndex, "backend index has the file");
      assert.ok(inTree, "file is indexed but missing from the tree / quick switcher");
    } finally {
      await app.close();
    }
  },
);
