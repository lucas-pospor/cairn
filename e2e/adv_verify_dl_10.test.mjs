// Regression tests for FINDING-010.
// "Save my version" on the "deleted or moved" banner used to force-write the
// path, even after something else had put a file there again.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_10.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Env, eventually, sleep, Key } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("verify-dl10");
});
after(async () => {
  await env?.dispose();
});

/** Every regular file under `dir` whose content contains `marker`. */
function filesContaining(dir, marker) {
  const out = [];
  const walk = (d) => {
    let ents;
    try {
      ents = fs.readdirSync(d, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of ents) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.isFile()) {
        try {
          if (fs.statSync(p).size < 1 << 20 && fs.readFileSync(p, "utf8").includes(marker)) out.push(p);
        } catch {}
      }
    }
  };
  walk(dir);
  return out;
}

test(
  "file deleted while typing, later recreated by an atomic replace (sync-client style): the recreated file survives Save my version",
  async () => {
    const v = env.vault("v", { "N.md": "original\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("N.md");
      await app.source();
      await sleep(150);
      // Real key events; the delete lands while the tab is still dirty.
      await app.typeEnd("MINE");
      v.rm("N.md");
      const b1 = await app.waitBanner();
      assert.match(b1, /deleted or moved/);
      // Some time later, another program puts a new N.md in place with
      // write-to-temp + rename (what sync clients do).
      await sleep(2000);
      const marker = `RECREATED-${process.pid}-${Date.now()}`;
      fs.writeFileSync(v.p(".N.md.tmp"), `${marker}\n`);
      fs.renameSync(v.p(".N.md.tmp"), v.p("N.md"));
      await sleep(3000); // far beyond the 250 ms watcher debounce
      const inTree = await app.exec(`return !!document.querySelector('[data-testid=tree-row][data-path="N.md"]')`);
      const b2 = await app.banner();
      await app.shot("verify-DL-10-stale-deleted-banner");
      console.log(`tree shows N.md again: ${inTree}; banner now: ${JSON.stringify(b2)}`);
      if (/Save my version/.test(b2 ?? "")) await app.clickButtonText("Save my version");
      await sleep(1500);
      const disk = v.read("N.md");
      const survivors = [
        ...filesContaining(env.tmp, marker),
        ...filesContaining(path.join(os.tmpdir(), `.Trash-${process.getuid()}`), marker),
        ...filesContaining(path.join(os.homedir(), ".local/share/Trash/files"), marker),
      ];
      console.log(`disk N.md=${JSON.stringify(disk)}; copies of the recreated text anywhere: ${JSON.stringify(survivors)}`);
      assert.ok(
        disk.includes(marker) || survivors.length > 0,
        `recreated file silently overwritten and not kept anywhere: banner was ${JSON.stringify(b2)}, disk=${JSON.stringify(disk)}`,
      );
    } finally {
      await app.close();
    }
  },
);

test(
  "in-app rename of a note that is NOT open onto a 'deleted' tab's path, then Save my version: the renamed note survives",
  async () => {
    const v = env.vault("v", { "X.md": "x original\n", "Y.md": "y content that matters\n" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("X.md");
      await app.source();
      await sleep(150);
      await app.typeEnd("MINE-X");
      v.rm("X.md");
      await app.waitBanner();
      // Rename Y (no tab open for it) to X from the tree.
      await app.exec(`document.querySelector('[data-testid=tree-row][data-path="Y.md"]').dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))`);
      await app.s.findWait("[data-testid=rename-input]");
      await app.exec(
        `const i = document.querySelector('[data-testid=rename-input]'); i.value = 'X'; i.dispatchEvent(new Event('input', { bubbles: true }));`,
      );
      await app.s.keys(Key.enter);
      await eventually(() => v.exists("X.md") && v.read("X.md") === "y content that matters\n", { message: "Y renamed to X" });
      await sleep(800);
      const tabs = await app.tabs();
      const b = await app.banner();
      console.log(`tabs=${JSON.stringify(tabs)} banner=${JSON.stringify(b)}`);
      if (/Save my version/.test(b ?? "")) await app.clickButtonText("Save my version");
      await sleep(1200);
      const all = v.listDisk().filter((p) => p.endsWith(".md")).map((p) => v.read(p));
      assert.ok(
        all.some((t) => t.includes("y content that matters")),
        `renamed note overwritten: disk X=${JSON.stringify(v.read("X.md"))}`,
      );
    } finally {
      await app.close();
    }
  },
);
