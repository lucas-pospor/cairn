// Adversarial tests: large vaults on the phone (3,000 notes) in app storage
// and in a shared folder opened through the system picker (SAF).
//
//   . scripts/android-env.sh
//   node --test --test-concurrency=1 e2e/android/adv_bigvault.test.mjs
//
// Needs one emulator in `adb devices` (the debug APK is installed if
// missing). Clears the app's data and uses /sdcard/Documents/AdvBigT.
// Prints the measured times.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { Device, adb, devSh, runAs, sleep, eventually, APK, PKG, q, ensureAppVault } from "./adv_helpers.mjs";

const N = 3000;
const LABEL = "AdvBigT";
const F = `/sdcard/Documents/${LABEL}`;
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-adv-an-big-"));
const src = path.join(tmp, "v");
const d = new Device();

before(async () => {
  if (!devSh(`pm list packages ${PKG}`).includes(PKG)) adb("install", "-r", APK);
  for (let i = 0; i < N; i++) {
    const dir = path.join(src, `F${i % 20}`);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, `Note ${i}.md`), `# Note ${i}\n\nSee [[Note ${(i * 7) % N}]] and #tag${i % 50}.\n${"Some body text for the note. ".repeat(20)}\n`);
  }
  devSh(`rm -rf ${F} && mkdir -p ${F} && echo seed > ${F}/Seed.md`);
  await d.fresh();
});

after(async () => {
  await d.shot("adv-bigvault-final.png").catch(() => {});
  d.close();
  devSh(`rm -rf ${F} /data/local/tmp/advbig`);
  fs.rmSync(tmp, { recursive: true, force: true });
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

const fileCount = () => d.eval(`window.__TAURI_INTERNALS__.invoke('list_entries').then(e => e.filter(x => x.kind === 'file').length)`);
const timed = (cmd, args = {}) => d.eval(`(async () => { const t = performance.now(); await window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)}); return Math.round(performance.now() - t); })()`);

test(`app storage: a ${N}-note vault opens in under 2 s and a rescan is quick`, async () => {
  const vaultDir = await ensureAppVault(d, "Big");
  await d.toWelcome();
  devSh("rm -rf /data/local/tmp/advbig && mkdir -p /data/local/tmp/advbig");
  execFileSync("adb", ["push", `${src}/.`, "/data/local/tmp/advbig"], { stdio: "ignore" });
  devSh("chmod -R a+rX /data/local/tmp/advbig");
  runAs(`rm -rf ${q(vaultDir)} && mkdir -p ${q(vaultDir)} && cp -r /data/local/tmp/advbig/. ${q(vaultDir)}/`);
  devSh("rm -rf /data/local/tmp/advbig");
  const t = Date.now();
  await d.eval(`[...document.querySelectorAll('.recent-open')].find(b => b.textContent.includes('Big')).click()`);
  await d.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 120000);
  const open = Date.now() - t;
  const count = await fileCount();
  const rescan = await timed("rescan");
  const search = await timed("search", { query: "body", limit: 100 });
  console.log(`app storage, ${N} notes: workspace after ${open} ms, ${count} files indexed, rescan ${rescan} ms, search ${search} ms`);
  assert.equal(count, N);
  assert.ok(open < 2000, `open took ${open} ms`);
  assert.ok(rescan < 1000, `rescan took ${rescan} ms`);
});

test(`SAF: a ${N}-note shared folder opens; the app shows progress while it loads and a second tap does not start a second load`, async () => {
  await d.launch();
  if (!(await d.isWelcome())) await d.toWelcome();
  await d.pickSafFolder(LABEL); // grant access while the folder is still small
  await d.toWelcome();
  execFileSync("adb", ["push", `${src}/.`, F], { stdio: "ignore" });
  adb("logcat", "-c");
  const t = Date.now();
  // The user taps the folder, then taps again 1.5 s later if the welcome
  // screen is still shown.
  await d.eval(`[...document.querySelectorAll('.recent-open')].find(b => b.textContent.includes(${JSON.stringify(LABEL)})).click()`);
  await sleep(1500);
  const feedback = await d.eval(`(() => { const t = document.body.innerText; return { welcome: !!document.querySelector('.welcome'), busy: !!document.querySelector('[aria-busy=true], .spinner, .loading, progress') || /opening|loading/i.test(t) }; })()`);
  await d.shot("adv-big-saf-opening.png");
  if (feedback.welcome) await d.eval(`[...document.querySelectorAll('.recent-open')].find(b => b.textContent.includes(${JSON.stringify(LABEL)}))?.click()`);
  await d.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 300000);
  const open = Date.now() - t;
  await sleep(15000); // let a second load (if any) finish and log
  const count = await fileCount();
  const rescan = await timed("rescan");
  const opens = devSh(`logcat -d | grep 'opened notebook' || true`).split("\n").filter((l) => l.includes(LABEL));
  console.log(`SAF, ${N} notes: workspace after ${open} ms, ${count} files indexed, rescan ${rescan} ms; 1.5 s after the tap: ${JSON.stringify(feedback)}; vault loads logged: ${opens.length}`);
  for (const l of opens) console.log("  " + l.replace(/^.*INFO\s+/, ""));
  assert.ok(count >= N, `${count} files indexed`);
  assert.ok(feedback.busy || !feedback.welcome, "something on screen shows that the folder is loading");
  assert.equal(opens.length, 1, "the second tap must not load the vault again");
});
