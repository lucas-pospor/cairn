// A file the server refuses (over CAIRN_MAX_BODY_MB) does not stop the other
// files, and the app says which file was left out: in the status bar and in
// Settings > Sync. Through the real app (FINDING-016).
//
// Run: scripts/e2e-headless.sh e2e/sync_skipped.test.mjs   (about 30 s)

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { Env, eventually, read, write, path, fs } from "./adv_sync_ui_lib.mjs";

const env = new Env("skipped");
const A = env.dir("laptop");
const P = env.dir("phone");
const PS = path.join(env.tmp, "phone-state");

before(async () => {
  write(path.join(A, "Start.md"), "start\n");
  // 900 KB is over the upload limit of a server with CAIRN_MAX_BODY_MB=1
  fs.mkdirSync(path.join(A, "attachments"));
  fs.writeFileSync(path.join(A, "attachments/clip.mp4"), crypto.randomBytes(900 << 10));
  write(path.join(A, "notes/today.md"), "written after the video\n");
  env.serverEnv = { CAIRN_MAX_BODY_MB: "1" };
  await env.startServer();
  await env.startApp(A);
  await env.connect({ server: env.surl });
});

after(async () => {
  await env.stop();
});

test("a file the server refuses is named in the status bar and in Settings; the other files sync", async () => {
  // (a sync started by the watcher may still be running)
  await eventually(async () => (await env.indicator()) === "Synced · 1 file not synced", { timeout: 20000, message: "the status bar says a file was not synced" });
  assert.match(await env.indicatorTitle(), /Not synced: attachments\/clip\.mp4\. /);
  await env.openSyncSettings();
  const rows = await env.s.exec(`return [...document.querySelectorAll('[data-testid=sync-skipped]')].map((r) => r.textContent.trim())`);
  const headings = await env.s.exec(`return [...document.querySelectorAll('[data-testid=settings] h4')].map((h) => h.textContent.trim())`);
  await env.closeSettings();
  // The same words as the status bar's "1 file not synced".
  assert.ok(headings.includes("Files not synced"), JSON.stringify(headings));
  assert.equal(rows.length, 1, JSON.stringify(rows));
  assert.match(rows[0], /^attachments\/clip\.mp4\s*the server does not accept files this large$/);

  const r = env.syncB(P, PS);
  assert.equal(r.error, undefined, JSON.stringify(r));
  assert.equal(read(path.join(P, "notes/today.md")), "written after the video\n");
  assert.ok(!fs.existsSync(path.join(P, "attachments/clip.mp4")));

  // once the file fits, it goes up and the notice goes away
  fs.writeFileSync(path.join(A, "attachments/clip.mp4"), crypto.randomBytes(100 << 10));
  await eventually(
    async () => {
      await env.syncNowFromUi();
      return (await env.indicator()) === "Synced";
    },
    { timeout: 30000, every: 1000, message: "the indicator is back to Synced" },
  );
  env.syncB(P, PS);
  assert.equal(fs.statSync(path.join(P, "attachments/clip.mp4")).size, 100 << 10);
});
