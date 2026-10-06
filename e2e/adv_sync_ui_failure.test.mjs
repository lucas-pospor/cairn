// A long note name arriving through sync, as the app shows it.
// The phone (sync_dir on Linux) uploads a note whose name is 245 bytes long,
// valid on its file system, plus an ordinary note. On the laptop the app
// used to fail to write it on every round (FINDING-050 at engine level: the
// temp file name got too long). Any error must name the note, and everything
// else must still sync.
//
// Run: scripts/e2e-headless.sh e2e/adv_sync_ui_failure.test.mjs   (about 40 s)

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Env, eventually, evidence, read, write, sleep, path, fs } from "./adv_sync_ui_lib.mjs";

const env = new Env("fail");
const A = env.dir("laptop");
const P = env.dir("phone");
const PS = path.join(env.tmp, "phone-state");
const LONG = "L".repeat(242) + ".md"; // 245 bytes

before(async () => {
  write(path.join(A, "Laptop.md"), "laptop note\n");
  await env.startServer();
  await env.startApp(A);
  await env.connect({ server: env.surl });
  await sleep(2500);
  const r = env.syncB(P, PS);
  assert.ok(!r.error, JSON.stringify(r));
});

after(async () => {
  await env.stop();
});

test(
  "an incoming note with a 245-byte name: any error names it, and every other change still syncs",
  async () => {
    write(path.join(P, LONG), "long name\n");
    write(path.join(P, "Fine.md"), "an ordinary note from the phone\n");
    const r = env.syncB(P, PS);
    assert.ok(!r.error, JSON.stringify(r));
    assert.equal(r.pushed, 2);
    // Laptop edits its own note, then syncs twice.
    write(path.join(A, "Laptop.md"), "laptop note, edited\n");
    await sleep(800);
    await env.syncNowFromUi();
    await env.syncNowFromUi();
    const ui = { indicator: await env.indicator(), title: await env.indicatorTitle() };
    await env.openSyncSettings();
    ui.settings = await env.s.exec(`return document.querySelector('[data-testid=settings]')?.innerText.split('Sync\\n').slice(-1)[0].slice(0, 500)`);
    await env.closeSettings();
    const fineArrived = read(path.join(A, "Fine.md"));
    env.syncB(P, PS);
    const laptopEditOnPhone = read(path.join(P, "Laptop.md"));
    const report = { ui, fineArrived, laptopEditOnPhone };
    evidence("SU-10.json", JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
    const problems = [];
    if (ui.indicator === "Sync error" && !ui.title?.includes(LONG.slice(0, 20))) problems.push(`error does not name the note: ${JSON.stringify(ui.title)}`);
    if (fineArrived === null) problems.push("Fine.md from the phone never arrived");
    if (laptopEditOnPhone !== "laptop note, edited\n") problems.push(`the laptop's edit did not reach the phone: ${JSON.stringify(laptopEditOnPhone)}`);
    assert.deepEqual(problems, []);
  },
);
