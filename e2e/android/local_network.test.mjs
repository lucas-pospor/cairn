// Sync to a server on the local network under Android 17, which blocks apps
// that target SDK 37 from local addresses unless the user allows "Nearby
// devices" (ACCESS_LOCAL_NETWORK). Without the permission a connection only
// timed out after about 30 s ("it did not answer in time"). The server runs
// on the host, which the emulator reaches at 10.0.2.2: an address on the
// emulator's own network, so Android blocks it like a server on a LAN.
//
//   . scripts/android-env.sh
//   emulator -avd <an Android 17 (API 37) AVD> &
//   node --test --test-concurrency=1 e2e/android/local_network.test.mjs
//
// Needs one emulator running Android 17 or later (the tests skip on older
// ones, where the permission does not exist), the debug APK (installed if
// missing) and target/debug/cairn-server. Clears the app's data and resets
// the permission.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Device, adb, devSh, runAs, appPid, eventually, APK, PKG, APP_DATA, startServer, fillSyncForm, ensureAppVault, writeAppFile, rescan, uiClick, uiDump } from "./adv_helpers.mjs";

const TOKEN = "lnp-token-0123456789abcdef";
const PASS = "local network passphrase";
const PERM = "android.permission.ACCESS_LOCAL_NETWORK";
const API = "/v1"; // cairn_sync::protocol::API_PREFIX
const sdk = Number(devSh("getprop ro.build.version.sdk").trim());
const skip = sdk < 37 ? `Android API ${sdk}: the local network permission exists from API 37` : false;
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-lnp-"));
const d = new Device();
let server;
let port;

before(async () => {
  if (skip) return;
  if (!devSh(`pm list packages ${PKG}`).includes(PKG)) adb("install", "-r", APK);
  ({ proc: server, port } = startServer(path.join(tmp, "server"), TOKEN));
  await eventually(async () => (await fetch(`http://127.0.0.1:${port}/health`)).ok, { message: "server up" });
  resetPermission();
  await d.fresh();
});

after(async () => {
  if (skip) return;
  await d.shot("lnp-final.png").catch(() => {});
  d.close();
  server?.kill();
  fs.rmSync(tmp, { recursive: true, force: true });
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

/**
 * Not granted, and Android may ask again, as on a fresh install: the app is
 * stopped (as a revoke in Android settings does), and Tauri's own record of
 * an earlier refusal is removed too. `fixed`: refused for good instead.
 */
function resetPermission({ fixed = false } = {}) {
  devSh(`am force-stop ${PKG}; pm revoke ${PKG} ${PERM}; pm clear-permission-flags ${PKG} ${PERM} user-set user-fixed`);
  runAs("rm -f shared_prefs/PluginPermStates.xml");
  if (fixed) devSh(`pm set-permission-flags ${PKG} ${PERM} user-set user-fixed`);
}

function granted() {
  return /granted=true/.test(devSh(`dumpsys package ${PKG} | grep '${PERM}:' || true`));
}

/** Whether the app's own uid can open a TCP connection to the server (what Android blocks). */
function appCanConnect() {
  try {
    devSh(`run-as ${PKG} toybox nc -w 5 10.0.2.2 ${port} </dev/null`);
    return true;
  } catch {
    return false;
  }
}

/** The vault the server has under `id`: HTTP status of GET /vaults/<id>. */
async function serverVault(id) {
  return (await fetch(`http://127.0.0.1:${port}${API}/vaults/${id}`, { headers: { authorization: `Bearer ${TOKEN}` } })).status;
}

const setupError = () => d.eval(`document.querySelector('[data-testid=sync-error]')?.textContent.trim() ?? null`);
const syncState = () => d.eval(`document.querySelector('[data-testid=sync-state]')?.textContent.trim() ?? null`);
const settingsError = () => d.eval(`document.querySelector('[data-testid=settings] .err')?.textContent.trim() ?? null`);
/** Android's permission prompt is on screen (not Cairn's own text, which names the setting too). */
const permissionPrompt = () => /package="com\.(google\.)?android\.permissioncontroller"/.test(uiDump());

test("the manifest declares the permission, and Android blocks the server without it", { skip }, async () => {
  const dump = devSh(`dumpsys package ${PKG}`);
  assert.match(dump, new RegExp(`requested permissions:[\\s\\S]*${PERM.replace(/\./g, "\\.")}`), "the APK must request ACCESS_LOCAL_NETWORK");
  assert.equal(granted(), false);
  // The emulator's host counts as the local network: the reason sync only
  // timed out before the fix.
  assert.equal(appCanConnect(), false, "Android should block the app's uid from 10.0.2.2 without the permission");
});

test("Connect and sync asks for Nearby devices; refusing stops at once with a message that names the setting", { skip }, async () => {
  await ensureAppVault(d, "LanSync");
  // A note for the version history test at the end.
  writeAppFile(`${APP_DATA}/vaults/LanSync/Lan note.md`, "written on the phone\n");
  await rescan(d);
  await fillSyncForm(d, { port, token: TOKEN, vaultId: "lan", device: "phone", pass: PASS });
  await uiClick(/^Don.t allow$/i, 20000);
  const t0 = Date.now();
  await d.waitFor(`!!document.querySelector('[data-testid=sync-error]')`, 20000);
  const took = Date.now() - t0;
  const err = await setupError();
  console.log(`refused: ${took} ms, error ${JSON.stringify(err)}`);
  await d.shot("lnp-refused.png");
  assert.ok(took < 15000, `the error should come at once, not after a timeout (${took} ms)`);
  assert.match(err, /10\.0\.2\.2 is on your local network/);
  assert.match(err, /"Nearby devices" for Cairn in Android settings/);
  assert.doesNotMatch(err, /did not answer in time/);
  assert.equal(await serverVault("lan"), 404, "nothing reached the server");
  assert.equal(granted(), false);
});

test("pressing Connect and sync again and allowing it connects and syncs", { skip }, async () => {
  await d.click("[data-testid=sync-connect]");
  await uiClick(/^Allow$/i, 20000);
  // The vault is new on the server: setup asks before it creates it.
  await d.waitFor(`(document.querySelector('[data-testid=dialog-ok]')?.click(), document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'idle')`, 60000);
  assert.equal(granted(), true);
  assert.equal(await serverVault("lan"), 200);
  assert.equal(appCanConnect(), true);
});

test("revoked in Android settings: the background sync says to press Sync now, which asks again", { skip }, async () => {
  // Android ends the app when a permission is revoked: Cairn keeps a grant
  // for the life of the process (android.rs, GRANTED) on that ground.
  const pid = appPid();
  assert.ok(pid, "the app runs, with the permission granted");
  devSh(`pm revoke ${PKG} ${PERM}`);
  await eventually(() => appPid() !== pid, { message: "Android ends the app when the permission is revoked" });
  resetPermission();
  await d.launch();
  // The first sync runs shortly after the vault opens; it must not ask by itself.
  await d.click("[data-testid=open-settings]");
  await d.waitFor(`!!document.querySelector('[data-testid=settings-sync]')`);
  await d.click("[data-testid=settings-sync]");
  await d.waitFor(`document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'error'`, 20000);
  const err = await settingsError();
  console.log(`background sync without the permission: ${JSON.stringify(err)}`);
  assert.equal(permissionPrompt(), false, "a sync the user did not start must not show Android's prompt");
  assert.match(err ?? "", /Press Sync now to allow it/);
  await d.click("[data-testid=sync-now]");
  await uiClick(/^Allow$/i, 20000);
  await d.waitFor(`document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'idle'`, 60000);
  assert.equal(granted(), true);
  assert.equal(await settingsError(), null);
});

test("refused for good: Sync now does not hang and points to Android settings", { skip }, async () => {
  resetPermission({ fixed: true });
  await d.launch();
  await d.click("[data-testid=open-settings]");
  await d.waitFor(`!!document.querySelector('[data-testid=settings-sync]')`);
  await d.click("[data-testid=settings-sync]");
  await d.waitFor(`document.querySelector('[data-testid=sync-state]')?.textContent.trim() === 'error'`, 20000);
  assert.match((await settingsError()) ?? "", /Press Sync now|Android settings/);
  const t0 = Date.now();
  await d.click("[data-testid=sync-now]");
  // Wait for this sync's error, not the one the background sync left.
  await d.waitFor(`/then sync again/.test(document.querySelector('[data-testid=settings] .err')?.textContent ?? '')`, 20000);
  const took = Date.now() - t0;
  const err = await settingsError();
  console.log(`Sync now, refused for good: ${took} ms, ${JSON.stringify(err)}`);
  await d.shot("lnp-refused-for-good.png");
  assert.equal(permissionPrompt(), false, "Android does not ask again");
  assert.ok(took < 15000, `${took} ms`);
  assert.match(err ?? "", /Allow "Nearby devices" for Cairn in Android settings \(Apps > Cairn > Permissions\), then sync again/);
  resetPermission();
});

test("version history asks for Nearby devices too, and loads once allowed", { skip }, async () => {
  resetPermission();
  await d.launch();
  // Ask for a note's history without waiting: Android's prompt comes first.
  const start = () =>
    d.eval(
      `(window.__hist = null, window.__TAURI_INTERNALS__.invoke("sync_history", { path: "Lan note.md" }).then((r) => (window.__hist = { ok: r.length }), (e) => (window.__hist = { err: String(e) })), true)`,
    );
  await start();
  await uiClick(/^Don.t allow$/i, 20000);
  await d.waitFor(`window.__hist !== null`, 20000);
  const refused = await d.eval(`window.__hist`);
  console.log(`history, refused: ${JSON.stringify(refused)}`);
  assert.match(refused.err ?? "", /10\.0\.2\.2 is on your local network/);
  assert.match(refused.err ?? "", /"Nearby devices" for Cairn in Android settings/);
  await start();
  await uiClick(/^Allow$/i, 20000);
  await d.waitFor(`window.__hist !== null`, 40000);
  const allowed = await d.eval(`window.__hist`);
  console.log(`history, allowed: ${JSON.stringify(allowed)}`);
  assert.equal(allowed.err, undefined, "the history loads from the server once allowed");
  assert.ok(allowed.ok >= 1, `the note's history has a version: ${JSON.stringify(allowed)}`);
  assert.equal(granted(), true);
});
