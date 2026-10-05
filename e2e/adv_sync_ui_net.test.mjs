// The connection drops in the middle of a running sync (after some uploads),
// then comes back. Through the real app.
//
// Run: scripts/e2e-headless.sh e2e/adv_sync_ui_net.test.mjs   (about 40 s)

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Env, eventually, evidence, read, write, sleep, path, fs } from "./adv_sync_ui_lib.mjs";

const env = new Env("net");
const A = env.dir("laptop");
const P = env.dir("phone");
const PS = path.join(env.tmp, "phone-state");
const N = 120;

before(async () => {
  write(path.join(A, "Start.md"), "start\n");
  await env.startServer();
  await env.startProxy();
  await env.startApp(A);
  await env.connect({ server: env.purl });
  await sleep(2500);
});

after(async () => {
  await env.stop();
});

test("connection lost after 40 of 120 uploads: 'Sync error', then a full recovery without duplicates (held up)", async () => {
  for (let i = 0; i < N; i++) write(path.join(A, `bulk/n${String(i).padStart(3, "0")}.md`), `bulk note ${i}\n`);
  await sleep(1000);
  env.postsLeft = 40;
  await env.syncNowFromUi({ timeout: 60000 });
  const down = { indicator: await env.indicator(), title: await env.indicatorTitle(), status: (await env.invoke("sync_status")).ok };
  env.postsLeft = undefined;
  env.proxyDown = false;
  await env.syncNowFromUi({ timeout: 60000 });
  const up = { indicator: await env.indicator(), status: (await env.invoke("sync_status")).ok };
  const r = env.syncB(P, PS);
  const pBulk = fs.existsSync(path.join(P, "bulk")) ? fs.readdirSync(path.join(P, "bulk")).sort() : [];
  const report = { down, up: { indicator: up.indicator, lastPushed: up.status.lastPushed, conflicts: up.status.conflicts }, phone: r, phoneBulkCount: pBulk.length };
  evidence("SU-net.json", JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  assert.equal(down.indicator, "Sync error");
  assert.match(down.title, /cannot reach the server/);
  assert.equal(up.indicator, "Synced");
  assert.equal(up.status.lastPushed, N - 40);
  assert.equal(pBulk.length, N);
  assert.deepEqual(r.conflicts, []);
});
