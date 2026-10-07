// Adversarial data-loss tests: autosave vs. external edits at many timings,
// and what the conflict banner's buttons really do.
//
// Run: scripts/e2e-headless.sh e2e/adv_dataloss.test.mjs
// Each test starts its own app on its own temp vault (start-up is ~0.3 s).

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { Env, eventually, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("timing");
});
after(async () => {
  await env?.dispose();
});

/** Start an app on a fresh vault and open `rel` in source mode. */
async function openFresh(files, rel) {
  const v = env.vault("v", files);
  const app = await env.launch(v);
  await app.openFromTree(rel);
  await app.source();
  await sleep(150);
  return { v, app };
}

test("external write before the autosave fires: banner, disk keeps theirs, editor keeps mine", async () => {
  const { v, app } = await openFresh({ "N.md": "base\n", "Z.md": "z\n" }, "N.md");
  try {
    await app.insertEnd("MINE");
    v.write("N.md", "THEIRS\n"); // well inside the 600 ms debounce
    await app.waitBanner();
    await sleep(800);
    assert.equal(v.read("N.md"), "THEIRS\n", "disk must keep the external version");
    assert.equal(await app.editorText(), "base\nMINE", "editor must keep my edits");
    assert.match(await app.saveState(), /conflict/);
  } finally {
    await app.close();
  }
});

test("external write right after an autosave: clean tab reloads, nothing is lost", async () => {
  const { v, app } = await openFresh({ "N.md": "base\n", "Z.md": "z\n" }, "N.md");
  try {
    await app.insertEnd("MINE");
    await eventually(() => v.read("N.md") === "base\nMINE", { message: "autosaved" });
    await app.waitSaved();
    v.write("N.md", "base\nMINE\nTHEIRS\n");
    await eventually(async () => (await app.editorText()) === "base\nMINE\nTHEIRS\n", { message: "editor reloaded" });
    assert.equal(await app.banner(), null);
    // and typing afterwards saves on top of their version
    await app.insertEnd("MORE");
    await eventually(() => v.read("N.md") === "base\nMINE\nTHEIRS\nMORE", { message: "saved on top" });
  } finally {
    await app.close();
  }
});

test("external write of identical content while dirty: no banner, my edit is saved", async () => {
  const { v, app } = await openFresh({ "N.md": "same\n", "Z.md": "z\n" }, "N.md");
  try {
    await app.insertEnd("MINE");
    v.write("N.md", "same\n");
    await eventually(() => v.read("N.md") === "same\nMINE", { message: "saved" });
    assert.equal(await app.banner(), null);
  } finally {
    await app.close();
  }
});

test("external edit of other lines while dirty: merged into my text and saved, no banner, typing goes on at the cursor", async () => {
  const { v, app } = await openFresh({ "N.md": "one\ntwo\nthree\nfour\nfive\n", "Z.md": "z\n" }, "N.md");
  try {
    await app.focusAt(13); // end of "three"
    await app.s.keys(" mine");
    v.write("N.md", "ONE\ntwo\nthree\nfour\nFIVE\n"); // inside the 600 ms debounce
    await eventually(() => v.read("N.md") === "ONE\ntwo\nthree mine\nfour\nFIVE\n", { message: "merged and saved" });
    assert.equal(await app.banner(), null);
    await app.s.keys("!");
    await eventually(() => v.read("N.md") === "ONE\ntwo\nthree mine!\nfour\nFIVE\n", { message: "typed at the cursor and saved" });
    await app.waitSaved();
  } finally {
    await app.close();
  }
});

test("external ABA (change then revert) inside the debounce: my edit is saved", async () => {
  const { v, app } = await openFresh({ "N.md": "aaa\n", "Z.md": "z\n" }, "N.md");
  try {
    await app.insertEnd("MINE");
    v.write("N.md", "bbb\n");
    v.write("N.md", "aaa\n");
    await eventually(() => v.read("N.md") === "aaa\nMINE", { message: "saved" });
    assert.equal(await app.banner(), null);
  } finally {
    await app.close();
  }
});

test("external truncation to empty while dirty: banner; Load disk version, then undo restores and saves mine", async () => {
  const { v, app } = await openFresh({ "N.md": "truncate me\n", "Z.md": "z\n" }, "N.md");
  try {
    await app.insertEnd("MINE");
    v.write("N.md", "");
    await app.waitBanner();
    assert.equal(v.read("N.md"), "");
    await app.clickTestId("conflict-theirs");
    await eventually(async () => (await app.editorText()) === "", { message: "disk version loaded" });
    assert.equal(await app.banner(), null);
    assert.equal(await app.saveState(), "Saved");
    // Undo brings my text back and it is saved (nothing was really lost).
    await app.exec(`document.querySelector('.cm-editor').__cairnView.focus()`);
    await app.s.keys({ chord: ["", "z"] });
    await eventually(() => v.read("N.md") === "truncate me\nMINE", { message: "undo saved" });
  } finally {
    await app.close();
  }
});

test("Keep mine (overwrite) writes exactly the editor text", async () => {
  const { v, app } = await openFresh({ "N.md": "base\n", "Z.md": "z\n" }, "N.md");
  try {
    await app.insertEnd("MINE");
    v.write("N.md", "THEIRS\n");
    await app.waitBanner();
    await app.insertEnd(" more"); // typing while in conflict must not save
    await sleep(900);
    assert.equal(v.read("N.md"), "THEIRS\n");
    await app.clickTestId("conflict-mine");
    await eventually(() => v.read("N.md") === "base\nMINE more", { message: "mine written" });
    // The banner goes when the write's reply comes back, a moment after the file changed.
    await eventually(async () => (await app.banner()) === null, { message: "banner gone" });
    await app.waitSaved();
  } finally {
    await app.close();
  }
});

test("hammer: external writes every few ms while typing; banner shown, every typed word kept", async () => {
  const { v, app } = await openFresh({ "H.md": "base\n", "Z.md": "z\n" }, "H.md");
  try {
    let n = 0;
    let stop = false;
    const writer = (async () => {
      while (!stop) {
        n++;
        fs.writeFileSync(v.p("H.md"), `EXT-${n}\nbase\n`);
        await sleep(7);
      }
    })();
    const typed = [];
    for (let i = 0; i < 10; i++) {
      typed.push(`U${i}`);
      await app.insertEnd(` U${i}`);
      await sleep(170 + (i % 4) * 160);
    }
    stop = true;
    await writer;
    await sleep(2000);
    const ed = await app.editorText();
    const disk = v.read("H.md");
    const missing = typed.filter((m) => !ed.includes(m) && !disk.includes(m));
    assert.deepEqual(missing, [], `typed words lost; editor=${JSON.stringify(ed)} disk=${JSON.stringify(disk)}`);
    if (!(await app.banner())) {
      // No conflict reported: then the editor and the disk must agree and
      // the last external write must not have been overwritten.
      assert.equal(ed, disk);
      assert.ok(disk.includes(`EXT-${n}`));
    } else {
      assert.equal(disk, `EXT-${n}\nbase\n`, "with a conflict pending the last external write stays on disk");
    }
  } finally {
    await app.close();
  }
});

test("external edit in a background tab with unsaved edits: conflict, disk untouched", async () => {
  const v = env.vault("v", { "A.md": "aaa\n", "B.md": "bbb\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("A.md");
    await app.source();
    await app.insertEnd("MINE");
    await app.openFromTree("B.md", { newTab: true }); // A goes to the background, still dirty
    v.write("A.md", "THEIRS\n");
    await sleep(1500);
    assert.equal(v.read("A.md"), "THEIRS\n");
    await app.exec(`[...document.querySelectorAll('[data-testid=tab]')].find(t => t.dataset.path === 'A.md').click()`);
    await app.waitBanner();
    assert.equal(await app.editorText(), "aaa\nMINE");
  } finally {
    await app.close();
  }
});

test("external touch (new mtime, same bytes) while dirty: no false conflict, my edit is saved", async () => {
  const { v, app } = await openFresh({ "N.md": "touched\n", "Z.md": "z\n" }, "N.md");
  try {
    await app.insertEnd("MINE");
    const now = new Date(Date.now() + 5000);
    fs.utimesSync(v.p("N.md"), now, now);
    await eventually(() => v.read("N.md") === "touched\nMINE", { message: "saved" });
    assert.equal(await app.banner(), null);
  } finally {
    await app.close();
  }
});

test(
  "same-size external edit that keeps the old mtime is noticed (or at least not overwritten)",
  async () => {
    // Models a coarse-mtime file system (FAT/exFAT USB stick, SMB, HFS+) or a
    // tool that preserves mtimes (rsync -t, cp -p, unzip, tar, sync clients).
    const { v, app } = await openFresh({ "A.md": "alpha one\n", "Z.md": "z\n" }, "A.md");
    try {
      await app.insertEnd("mine1 ");
      await eventually(() => v.read("A.md") === "alpha one\nmine1 ", { message: "first autosave" });
      await app.waitSaved();
      await sleep(700); // let the watcher digest Cairn's own write
      const st = fs.statSync(v.p("A.md"), { bigint: true });
      const ms = Number(st.mtimeNs / 1000000n); // what Cairn indexed (whole milliseconds)
      fs.writeFileSync(v.p("A.md"), "ALPHA one\nmine1 "); // same size, other program
      // Put the mtime back inside the same millisecond (middle of it, so
      // float rounding in utimes cannot cross into the neighbouring one).
      fs.utimesSync(v.p("A.md"), (ms + 0.5) / 1000, (ms + 0.5) / 1000);
      assert.equal(Number(fs.statSync(v.p("A.md"), { bigint: true }).mtimeNs / 1000000n), ms, "test setup: mtime restored");
      await sleep(1200);
      const reloaded = (await app.editorText()).startsWith("ALPHA");
      await app.insertEnd("mine2");
      await sleep(1500);
      const disk = v.read("A.md");
      const banner = await app.banner();
      assert.ok(
        reloaded || banner || disk.includes("ALPHA"),
        `external edit silently overwritten: reloaded=${reloaded} banner=${banner} disk=${JSON.stringify(disk)}`,
      );
    } finally {
      await app.close();
    }
  },
);

test(
  "external write that lands while Cairn is writing its temp file is not silently clobbered",
  async () => {
    // ~2 MB note so the window between Cairn's hash check and its rename is a
    // few ms; the race also hits small notes (1 in 5 attempts in testing).
    const big = "lorem ipsum dolor sit amet\n".repeat(80_000);
    const { v, app } = await openFresh({ "Big.md": big, "Z.md": "z\n" }, "Big.md");
    let w;
    try {
      let hit = false;
      w = fs.watch(v.root, (_ev, name) => {
        if (!hit && name && name.startsWith(".cairn-tmp-")) {
          hit = true; // Cairn has passed its base-hash check and is writing
          fs.writeFileSync(v.p("Big.md"), "EXTERNAL WRITE DURING SAVE\n");
        }
      });
      await app.insertAt(0, "MINE ");
      await eventually(async () => (await app.saveState()) !== "Unsaved", { timeout: 20000, message: "save finished" });
      await sleep(1500);
      w.close();
      assert.ok(hit, "the external write was not triggered (temp file not seen)");
      const disk = v.read("Big.md");
      const banner = await app.banner();
      assert.ok(
        disk.includes("EXTERNAL WRITE DURING SAVE") || banner,
        `external write lost without a conflict: disk starts ${JSON.stringify(disk.slice(0, 30))}, banner=${banner}`,
      );
    } finally {
      w?.close();
      await app.close();
    }
  },
);
