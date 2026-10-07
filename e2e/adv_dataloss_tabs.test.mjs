// Adversarial data-loss tests: tabs, closing, navigating, switching vaults,
// closing the window, renames and deletes while a tab has unsaved edits.
//
// Run: scripts/e2e-headless.sh e2e/adv_dataloss_tabs.test.mjs
// Each test starts its own app on its own temp vault.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { Env, eventually, sleep, Key } from "./adv_dataloss_helpers.mjs";
import { pickThemeMode } from "./theme_mode.mjs";

let env;
before(async () => {
  env = await Env.create("tabs");
});
after(async () => {
  await env?.dispose();
});

async function openFresh(files, rel) {
  const v = env.vault("v", files);
  const app = await env.launch(v);
  await app.openFromTree(rel);
  await app.source();
  await sleep(150);
  return { v, app };
}

/** Put the active tab (rel, edited) into the "changed on disk" conflict state. */
async function makeConflict(app, v, rel, mine) {
  await app.insertEnd(mine);
  v.write(rel, "THEIRS\n");
  await app.waitBanner();
}

async function runCommand(app, name) {
  await app.s.keys({ chord: [Key.ctrl, "p"] });
  await app.s.type(await app.s.findWait("[data-testid=palette-input]"), name);
  await sleep(200);
  await app.s.keys(Key.enter);
}

async function renameInTree(app, from, newName) {
  await app.exec(`document.querySelector('[data-testid=tree-row][data-path="${from}"]').dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))`);
  await app.s.findWait("[data-testid=rename-input]");
  await app.exec(
    `const i = document.querySelector('[data-testid=rename-input]'); i.value = arguments[0]; i.dispatchEvent(new Event('input', { bubbles: true }));`,
    newName,
  );
  await app.s.keys(Key.enter);
}

/** Wait for the dialog whose text matches `re`, then click its `label` button. */
async function answerDialog(app, re, label) {
  await app.s.waitFor(`return ${re}.test(document.querySelector('.dialog')?.textContent ?? '')`, { message: `dialog ${re}` });
  await app.exec(`[...document.querySelectorAll('.dialog button')].find((b) => b.textContent.trim() === arguments[0]).click()`, label);
}

// ---------- things that held up ----------

test("external delete while dirty: banner; Save my version recreates the file with my text", async () => {
  const { v, app } = await openFresh({ "N.md": "base\n", "Z.md": "z\n" }, "N.md");
  try {
    await app.insertEnd("MINE");
    v.rm("N.md");
    const b = await app.waitBanner();
    assert.match(b, /deleted or moved/);
    assert.ok(!v.exists("N.md"), "nothing recreated without asking");
    await app.clickButtonText("Save my version");
    await eventually(() => v.exists("N.md") && v.read("N.md") === "base\nMINE", { message: "recreated" });
    // The banner goes when the write's reply comes back, a moment after the file changed.
    await eventually(async () => (await app.banner()) === null, { message: "banner gone" });
  } finally {
    await app.close();
  }
});

test("external delete while dirty: Discard and close closes the tab and writes nothing", async () => {
  const { v, app } = await openFresh({ "N.md": "base\n", "Z.md": "z\n" }, "N.md");
  try {
    await app.insertEnd("MINE");
    v.rm("N.md");
    await app.waitBanner();
    await app.clickButtonText("Discard and close");
    await eventually(async () => (await app.tabs()).length === 0, { message: "tab closed" });
    await sleep(800);
    assert.ok(!v.exists("N.md"));
  } finally {
    await app.close();
  }
});

test("external rename while dirty: tab follows and the edits are saved to the new path only", async () => {
  const { v, app } = await openFresh({ "B.md": "bravo\n", "Z.md": "z\n" }, "B.md");
  try {
    await app.insertEnd("MINE");
    v.mv("B.md", "B2.md");
    await eventually(() => v.read("B2.md") === "bravo\nMINE", { message: "saved to new path" });
    assert.ok(!v.exists("B.md"), "old path must not be recreated");
    assert.equal(await app.activeTab(), "B2.md");
    assert.equal(await app.banner(), null);
  } finally {
    await app.close();
  }
});

test("external folder rename while dirty (no other recent activity): tab follows, edits land in the new folder", async () => {
  const v = env.vault("v", { "dir/C.md": "charlie\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("dir/C.md");
    await app.source();
    await sleep(700); // let the open's own fs events drain
    await app.insertEnd("MINE");
    v.mv("dir", "dir2");
    await eventually(() => v.read("dir2/C.md") === "charlie\nMINE", { message: "saved in dir2" });
    assert.ok(!v.exists("dir"), "old folder must not come back");
    assert.equal(await app.activeTab(), "dir2/C.md");
  } finally {
    await app.close();
  }
});

test("in-app rename within the autosave debounce: edits flushed, then renamed", async () => {
  const { v, app } = await openFresh({ "A.md": "alpha\n", "Z.md": "z\n" }, "A.md");
  try {
    await app.insertEnd("MINE");
    await renameInTree(app, "A.md", "A renamed");
    await eventually(() => v.exists("A renamed.md") && v.read("A renamed.md") === "alpha\nMINE", { message: "renamed with edits" });
    assert.ok(!v.exists("A.md"));
    assert.equal(await app.activeTab(), "A renamed.md");
  } finally {
    await app.close();
  }
});

test("in-app folder rename with a dirty tab inside: edits flushed and the tab follows", async () => {
  const v = env.vault("v", { "dir/C.md": "charlie\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("dir/C.md");
    await app.source();
    await app.insertEnd("MINE");
    await renameInTree(app, "dir", "dir2");
    await eventually(() => v.exists("dir2/C.md") && v.read("dir2/C.md") === "charlie\nMINE", { message: "folder renamed with edits" });
    assert.ok(!v.exists("dir"));
    await eventually(async () => (await app.activeTab()) === "dir2/C.md", { message: "tab follows" });
  } finally {
    await app.close();
  }
});

test("Switch vault right after typing: the edit is flushed to the old vault, the new vault is untouched", async () => {
  const v1 = env.vault("one", { "A.md": "one\n" });
  const v2 = env.vault("two", { "A.md": "two\n" });
  const app = await env.launch(v1);
  try {
    await app.openFromTree("A.md");
    await app.source();
    await app.typeEnd("TYPED");
    await runCommand(app, "Switch notebook");
    await app.s.findWait("[data-testid=vault-path]");
    await app.s.type(await app.s.find("[data-testid=vault-path]"), v2.root);
    await app.s.click(await app.s.find("[data-testid=vault-open]"));
    await app.s.findWait("[data-testid=file-tree]");
    await sleep(1200);
    assert.equal(v1.read("A.md"), "one\nTYPED");
    assert.equal(v2.read("A.md"), "two\n");
  } finally {
    await app.close();
  }
});

test("save into a read-only folder fails visibly and keeps the edits in the tab", async () => {
  const v = env.vault("v", { "d/A.md": "alpha\n", "B.md": "b\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("d/A.md");
    await app.source();
    fs.chmodSync(v.p("d"), 0o555);
    await app.insertEnd("MINE");
    await eventually(async () => (await app.toasts()).some((t) => /Could not save/.test(t)), { message: "error toast" });
    assert.equal(await app.saveState(), "Unsaved");
    assert.equal(await app.editorText(), "alpha\nMINE");
    assert.equal(v.read("d/A.md"), "alpha\n");
    // Once the folder is writable again, the next edit saves everything.
    fs.chmodSync(v.p("d"), 0o755);
    await app.insertEnd("!");
    await eventually(() => v.read("d/A.md") === "alpha\nMINE!", { message: "saved after fix" });
  } finally {
    fs.chmodSync(v.p("d"), 0o755);
    await app.close();
  }
});

// ---------- findings ----------

test(
  "conflict banner shown, then a plain click on another note in the tree keeps my unsaved edits",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n", "B.md": "bravo\n" }, "A.md");
    try {
      await makeConflict(app, v, "A.md", "MY IMPORTANT EDIT");
      await app.shot("DL-01-before-click");
      await app.openFromTree("B.md"); // the user just looks at another note
      await app.openFromTree("A.md");
      await app.shot("DL-01-after-coming-back");
      const ed = await app.editorText();
      const where = [ed, v.read("A.md"), ...v.listDisk().filter((p) => p.endsWith(".md")).map((p) => v.read(p))];
      assert.ok(
        where.some((t) => t.includes("MY IMPORTANT EDIT")),
        `edit gone: editor=${JSON.stringify(ed)} disk A=${JSON.stringify(v.read("A.md"))}`,
      );
    } finally {
      await app.close();
    }
  },
);

test(
  "save failed (read-only folder), then a plain click on another note keeps my unsaved edits",
  async () => {
    const v = env.vault("v", { "d/A.md": "alpha\n", "B.md": "bravo\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("d/A.md");
      await app.source();
      fs.chmodSync(v.p("d"), 0o555);
      await app.insertEnd("MY IMPORTANT EDIT");
      await eventually(async () => (await app.toasts()).some((t) => /Could not save/.test(t)), { message: "error toast" });
      await app.openFromTree("B.md");
      fs.chmodSync(v.p("d"), 0o755);
      await app.openFromTree("d/A.md");
      const ed = await app.editorText();
      assert.ok(
        ed.includes("MY IMPORTANT EDIT") || v.read("d/A.md").includes("MY IMPORTANT EDIT"),
        `edit gone: editor=${JSON.stringify(ed)} disk=${JSON.stringify(v.read("d/A.md"))}`,
      );
    } finally {
      fs.chmodSync(v.p("d"), 0o755);
      await app.close();
    }
  },
);

test(
  "closing a tab that shows the conflict banner asks first (or keeps the edits)",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n", "Z.md": "z\n" }, "A.md");
    try {
      await makeConflict(app, v, "A.md", "MY IMPORTANT EDIT");
      await app.shot("DL-02-before-close");
      await app.closeActiveTab();
      await sleep(500);
      const asked = await app.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);
      const stillOpen = (await app.tabs()).some((t) => t.path === "A.md");
      assert.ok(asked || stillOpen || v.read("A.md").includes("MY IMPORTANT EDIT"), "tab closed and the edits are gone, no prompt");
    } finally {
      await app.close();
    }
  },
);

test(
  "Ctrl+W on a tab whose save failed asks first (or keeps the edits)",
  async () => {
    const v = env.vault("v", { "d/A.md": "alpha\n", "B.md": "b\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("d/A.md");
      await app.source();
      fs.chmodSync(v.p("d"), 0o555);
      await app.insertEnd("MY IMPORTANT EDIT");
      await eventually(async () => (await app.toasts()).some((t) => /Could not save/.test(t)), { message: "error toast" });
      await app.focusEnd();
      await app.s.keys({ chord: [Key.ctrl, "w"] });
      await sleep(500);
      const asked = await app.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);
      const stillOpen = (await app.tabs()).some((t) => t.path === "d/A.md");
      assert.ok(asked || stillOpen, "tab closed and the unsaved edits are gone, no prompt");
    } finally {
      fs.chmodSync(v.p("d"), 0o755);
      await app.close();
    }
  },
);

test(
  "Switch vault with a conflicted tab asks first (or keeps the edits)",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n", "Z.md": "z\n" }, "A.md");
    try {
      await makeConflict(app, v, "A.md", "MY IMPORTANT EDIT");
      await runCommand(app, "Switch notebook");
      await sleep(800);
      const asked = await app.exec(`return !!document.querySelector('[data-testid=dialog-ok]')`);
      const stillInVault = await app.exec(`return !!document.querySelector('[data-testid=file-tree]')`);
      assert.ok(asked || stillInVault || v.read("A.md").includes("MY IMPORTANT EDIT"), "vault closed and the edits are gone, no prompt");
    } finally {
      await app.close();
    }
  },
);

test("conflict banner, then opening another note keeps the edited tab and opens the note next to it", async () => {
  const { v, app } = await openFresh({ "A.md": "alpha\n", "B.md": "bravo\n" }, "A.md");
  try {
    await makeConflict(app, v, "A.md", "MY IMPORTANT EDIT");
    await app.openFromTree("B.md");
    assert.deepEqual(await app.tabs(), [
      { path: "A.md", dirty: true, active: false },
      { path: "B.md", dirty: false, active: true },
    ]);
    assert.equal(v.read("A.md"), "THEIRS\n", "nothing was overwritten");
  } finally {
    await app.close();
  }
});

test("closing a tab whose save is blocked: Cancel keeps the tab and its edits, Discard closes it and writes nothing", async () => {
  const { v, app } = await openFresh({ "A.md": "alpha\n", "Z.md": "z\n" }, "A.md");
  try {
    await makeConflict(app, v, "A.md", "MY IMPORTANT EDIT");
    await app.closeActiveTab();
    await answerDialog(app, /Discard unsaved changes to "A"\?/, "Cancel");
    await sleep(300);
    assert.deepEqual((await app.tabs()).map((t) => t.path), ["A.md"]);
    assert.equal(await app.editorText(), "alpha\nMY IMPORTANT EDIT");
    assert.match((await app.banner()) ?? "", /changed on disk/);
    await app.closeActiveTab();
    await answerDialog(app, /Discard unsaved changes to "A"\?/, "Discard");
    await eventually(async () => (await app.tabs()).length === 0, { message: "tab closed" });
    await sleep(800);
    assert.equal(v.read("A.md"), "THEIRS\n");
  } finally {
    await app.close();
  }
});

test("Switch vault and the status-bar vault button with a blocked save: Cancel stays in the vault with the edits", async () => {
  const { v, app } = await openFresh({ "A.md": "alpha\n", "Z.md": "z\n" }, "A.md");
  try {
    await makeConflict(app, v, "A.md", "MY IMPORTANT EDIT");
    await app.openFromTree("Z.md", { newTab: true });
    await runCommand(app, "Switch notebook");
    await answerDialog(app, /Discard unsaved changes to "A"\?/, "Cancel");
    await sleep(300);
    assert.ok(await app.exec(`return !!document.querySelector('[data-testid=file-tree]')`), "still in the vault");
    assert.equal(await app.activeTab(), "A.md", "the tab with the unsaved edits is shown");
    assert.equal(await app.editorText(), "alpha\nMY IMPORTANT EDIT");
    await app.s.click(await app.s.find('button.vault[title="Switch notebook"]'));
    await answerDialog(app, /Discard unsaved changes to "A"\?/, "Cancel");
    await sleep(300);
    assert.ok(await app.exec(`return !!document.querySelector('[data-testid=file-tree]')`), "still in the vault");
    await app.s.click(await app.s.find('button.vault[title="Switch notebook"]'));
    await answerDialog(app, /Discard unsaved changes to "A"\?/, "Discard");
    await app.s.findWait("[data-testid=vault-path]");
    assert.equal(v.read("A.md"), "THEIRS\n");
  } finally {
    await app.close();
  }
});

test(
  "closing the window right after typing still saves the edit",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n" }, "A.md");
    try {
      await app.typeEnd(" last words");
      await app.closeWindow();
      await sleep(2000);
      await assert.rejects(app.exec("return 1"), "the window is gone");
      assert.equal(v.read("A.md"), "alpha\n last words");
    } finally {
      await app.close();
    }
  },
);

test("closing the window with a blocked save asks first: Cancel keeps the window and the edits, Discard closes it", async () => {
  const { v, app } = await openFresh({ "A.md": "alpha\n", "Z.md": "z\n" }, "A.md");
  try {
    await makeConflict(app, v, "A.md", "MY IMPORTANT EDIT");
    await app.closeWindow();
    await answerDialog(app, /Discard unsaved changes to "A"\?/, "Cancel");
    await sleep(500);
    assert.equal(await app.editorText(), "alpha\nMY IMPORTANT EDIT");
    assert.match((await app.banner()) ?? "", /changed on disk/);
    await app.closeWindow();
    await answerDialog(app, /Discard unsaved changes to "A"\?/, "Discard");
    await sleep(1500);
    await assert.rejects(app.exec("return 1"), "the window is gone");
    assert.equal(v.read("A.md"), "THEIRS\n");
  } finally {
    await app.close();
  }
});

test("closing the window when the settings cannot be saved asks first; Cancel keeps the window open", async () => {
  const { v, app } = await openFresh({ "A.md": "alpha\n" }, "A.md");
  const lock = (mode) => {
    if (fs.existsSync(v.p(".cairn/settings.json"))) fs.chmodSync(v.p(".cairn/settings.json"), mode & 0o666);
    fs.chmodSync(v.p(".cairn"), mode);
  };
  try {
    await app.exec(`document.querySelector('[data-testid=open-settings]').click(); return 1`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=theme-select]')`);
    fs.mkdirSync(v.p(".cairn"), { recursive: true });
    lock(0o555);
    await app.exec(`${pickThemeMode("dark")} return 1`);
    await app.closeWindow();
    await answerDialog(app, /Discard unsaved changes to the settings\?/, "Cancel");
    lock(0o755);
    await sleep(300);
    assert.equal(await app.exec(`return document.documentElement.dataset.theme ?? null`), "dark", "the window is still open");
    await app.closeWindow();
    await sleep(1500);
    await assert.rejects(app.exec("return 1"), "the window is gone");
    assert.equal(JSON.parse(v.read(".cairn/settings.json")).theme, "dark");
  } finally {
    try {
      lock(0o755);
    } catch {}
    await app.close();
  }
});

test(
  "'deleted' banner, file comes back from another program: Save my version must not silently overwrite it",
  async () => {
    const { v, app } = await openFresh({ "N.md": "original\n", "Z.md": "z\n" }, "N.md");
    try {
      await app.insertEnd("MINE");
      v.rm("N.md"); // e.g. git stash / a sync client replacing the file
      await app.waitBanner();
      v.write("N.md", "RECREATED BY OTHER PROGRAM\n");
      await sleep(1500);
      await app.shot("DL-10-banner-after-file-came-back");
      const banner = await app.banner();
      if (/Save my version/.test(banner ?? "")) await app.clickButtonText("Save my version");
      await sleep(1000);
      const disk = v.read("N.md");
      assert.ok(
        disk.includes("RECREATED BY OTHER PROGRAM") || !/deleted or moved/.test(banner ?? ""),
        `banner still said "deleted" after the file came back, and Save my version replaced it: disk=${JSON.stringify(disk)}`,
      );
    } finally {
      await app.close();
    }
  },
);

test(
  "renaming another note onto a 'deleted' tab's path, then Save my version, does not destroy that note",
  async () => {
    const { v, app } = await openFresh({ "X.md": "x original\n", "Y.md": "y content that matters\n" }, "X.md");
    try {
      await app.insertEnd("MINE-X");
      v.rm("X.md");
      await app.waitBanner();
      await app.openFromTree("Y.md", { newTab: true });
      await renameInTree(app, "Y.md", "X");
      await eventually(() => v.exists("X.md") && v.read("X.md") === "y content that matters\n", { message: "Y renamed to X" });
      const tabs = await app.tabs();
      await app.exec(`[...document.querySelectorAll('[data-testid=tab]')][0].click()`);
      await sleep(300);
      if (/Save my version/.test((await app.banner()) ?? "")) await app.clickButtonText("Save my version");
      await sleep(1000);
      const all = v.listDisk().filter((p) => p.endsWith(".md")).map((p) => v.read(p));
      assert.ok(
        all.some((t) => t.includes("y content that matters")),
        `renamed note overwritten; tabs were ${JSON.stringify(tabs)}; disk X=${JSON.stringify(v.read("X.md"))}`,
      );
    } finally {
      await app.close();
    }
  },
);

test("'deleted' banner, then a folder renamed onto the note's folder: Save my version leaves the note there and says 'changed'", async () => {
  // A folder rename reports only the folder, so the banner still says
  // "deleted"; the write itself must refuse to replace the file.
  const { v, app } = await openFresh({ "F/a.md": "a original\n", "H/a.md": "h content that matters\n" }, "F/a.md");
  try {
    await app.insertEnd("MINE");
    v.rm("F");
    assert.match(await app.waitBanner(), /deleted or moved/);
    // (a file list refresh while the rename box is open resets its text)
    await app.s.waitFor(`return !document.querySelector('[data-testid=tree-row][data-path="F"]')`, { message: "F gone from the tree" });
    await renameInTree(app, "H", "F");
    await eventually(() => v.exists("F/a.md") && v.read("F/a.md") === "h content that matters\n", { message: "H renamed to F" });
    await app.exec(`[...document.querySelectorAll('[data-testid=tab]')][0].click()`);
    await sleep(300);
    if (/Save my version/.test((await app.banner()) ?? "")) await app.clickButtonText("Save my version");
    await eventually(async () => /changed on disk/.test((await app.banner()) ?? ""), { message: "banner says changed" });
    assert.equal(v.read("F/a.md"), "h content that matters\n");
    assert.equal(await app.editorText(), "a original\nMINE");
  } finally {
    await app.close();
  }
});

test("opening a note that is already open activates its tab (no second copy); split view edits save", async () => {
  const v = env.vault("v", { "A.md": "alpha\n", "B.md": "bravo\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("A.md");
    await app.openFromTree("B.md", { newTab: true });
    await app.openFromTree("A.md", { newTab: true });
    assert.deepEqual((await app.tabs()).map((t) => t.path), ["A.md", "B.md"]);
    await app.clickTestId("mode-split");
    await app.insertEnd("**split edit**");
    await eventually(() => v.read("A.md") === "alpha\n**split edit**", { message: "saved from split view" });
    await app.s.waitFor(`return document.querySelector('[data-testid=preview]')?.innerHTML.includes('<strong>split edit</strong>')`, { timeout: 5000 });
  } finally {
    await app.close();
  }
});

test(
  "deleting a note in the app right after typing puts the latest text in the trash",
  async () => {
    const v = env.vault("v", { "Del.md": "saved text\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("Del.md");
      await app.source();
      await app.insertEnd("typed just now");
      await app.exec(`document.querySelector('[data-testid=file-tree]').focus()`);
      await app.s.keys(Key.delete);
      await app.s.click(await app.s.findWait("[data-testid=dialog-ok]"));
      await eventually(() => !v.exists("Del.md"), { message: "deleted" });
      await sleep(800);
      const trashDir = `${env.tmp}/data/Trash/files`;
      const copies = fs.existsSync(trashDir) ? fs.readdirSync(trashDir).filter((f) => f.startsWith("Del")).map((f) => fs.readFileSync(`${trashDir}/${f}`, "utf8")) : [];
      const vaultTrash = v.exists(".trash") ? fs.readdirSync(v.p(".trash")).map((f) => v.read(`.trash/${f}`)) : [];
      const all = [...copies, ...vaultTrash];
      assert.ok(all.length > 0, "no trash copy found");
      assert.ok(all.some((t) => t.includes("typed just now")), `trash has only ${JSON.stringify(all)}`);
    } finally {
      await app.close();
    }
  },
);

test(
  "deleting a note that cannot be deleted (read-only folder) keeps its tab and unsaved edits",
  async () => {
    const v = env.vault("v", { "ro/Del.md": "saved text\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("ro/Del.md");
      await app.source();
      await app.insertEnd(" typed");
      await eventually(() => v.read("ro/Del.md") === "saved text\n typed", { message: "autosaved" });
      fs.chmodSync(v.p("ro"), 0o555);
      await app.insertEnd(" UNSAVED");
      await app.exec(`document.querySelector('[data-testid=tree-row][data-path="ro/Del.md"]').click()`);
      await app.exec(`document.querySelector('[data-testid=file-tree]').focus()`);
      await app.s.keys(Key.delete);
      await app.s.click(await app.s.findWait("[data-testid=dialog-ok]"));
      await sleep(1000);
      const toasts = await app.toasts();
      const tabs = await app.tabs();
      assert.ok(v.exists("ro/Del.md"), "precondition: the delete failed");
      assert.ok(
        tabs.some((t) => t.path === "ro/Del.md"),
        `delete failed (${JSON.stringify(toasts)}) but the tab with unsaved " UNSAVED" was already closed; disk=${JSON.stringify(v.read("ro/Del.md"))}`,
      );
    } finally {
      fs.chmodSync(v.p("ro"), 0o755);
      await app.close();
    }
  },
);

test("deleting a note whose save is blocked asks about the unsaved edits; Cancel or a failed delete keeps them", async () => {
  const v = env.vault("v", { "ro/Del.md": "saved text\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("ro/Del.md");
    await app.source();
    fs.chmodSync(v.p("ro"), 0o555);
    await app.insertEnd(" UNSAVED");
    await runCommand(app, "Delete current note");
    await answerDialog(app, /Move "Del" to the trash\?/, "Delete");
    await answerDialog(app, /Discard unsaved changes to "Del"\?/, "Cancel");
    await sleep(300);
    assert.ok(v.exists("ro/Del.md"), "Cancel deletes nothing");
    assert.deepEqual((await app.tabs()).map((t) => t.path), ["ro/Del.md"]);
    assert.equal(await app.editorText(), "saved text\n UNSAVED");
    // Discard, but the delete itself fails: the tab and its edits stay.
    await runCommand(app, "Delete current note");
    await answerDialog(app, /Move "Del" to the trash\?/, "Delete");
    await answerDialog(app, /Discard unsaved changes to "Del"\?/, "Discard");
    await eventually(async () => (await app.toasts()).some((t) => /No permission to access "ro\/Del\.md"\./.test(t) && !/Could not save/.test(t)), { message: "delete error toast" });
    assert.ok(v.exists("ro/Del.md"));
    assert.deepEqual((await app.tabs()).map((t) => t.path), ["ro/Del.md"]);
    assert.equal(await app.editorText(), "saved text\n UNSAVED");
  } finally {
    fs.chmodSync(v.p("ro"), 0o755);
    await app.close();
  }
});
