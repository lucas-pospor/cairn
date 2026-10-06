// Adversarial tests: sync on the phone (app-storage and SAF vaults) against a
// cairn-server on the host (reachable from the emulator at 10.0.2.2), with
// target/debug/examples/sync_dir as a second ("desktop") device.
//
//   . scripts/android-env.sh
//   node --test --test-concurrency=1 e2e/android/adv_sync.test.mjs
//
// Needs one emulator in `adb devices` with the debug APK installed (it is
// installed if missing), the prebuilt target/debug/cairn-server and
// target/debug/examples/sync_dir. Clears the app's data and uses
// /sdcard/Documents/AdvSyncT on the device.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import {
  Device, adb, devSh, sleep, eventually, appPid, runAs, key, APK, APP_DATA, PKG, q, ls,
  startServer, syncDir, setupSync, fillSyncForm, closeSettings, ensureAppVault, ensureSafVault, reopenSaf, rescan, writeAppFile, amKill, kill9,
} from "./adv_helpers.mjs";

const TOKEN = "adv-sync-token-0123456789abcdef";
const PASS = "adv sync passphrase";
const LABEL = "AdvSyncT";
const API = "/v1"; // cairn_sync::protocol::API_PREFIX
const F = `/sdcard/Documents/${LABEL}`;
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-adv-an-sync-"));
const d = new Device();
let server;
let port;

before(async () => {
  if (!devSh(`pm list packages ${PKG}`).includes(PKG)) adb("install", "-r", APK);
  ({ proc: server, port } = startServer(path.join(tmp, "server"), TOKEN));
  await eventually(async () => (await fetch(`http://127.0.0.1:${port}/health`)).ok, { message: "server up" });
  devSh(`mkdir -p ${F} && cd ${F} && rm -rf ./* ./.trash ./.cairn`);
  await d.fresh();
});

after(async () => {
  await d.shot("adv-sync-final.png").catch(() => {});
  d.close();
  server?.kill();
  fs.rmSync(tmp, { recursive: true, force: true });
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

const invoke = (cmd, args = {}) => d.eval(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)})`);

/** A "desktop" device: a temp folder synced with sync_dir. */
function desktop(vaultId) {
  const dir = fs.mkdtempSync(path.join(tmp, `desk-${vaultId}-`));
  const vault = path.join(dir, "vault");
  const state = path.join(dir, "state");
  fs.mkdirSync(vault);
  return {
    vault,
    sync: () => syncDir(vault, state, port, TOKEN, vaultId, "desktop", PASS),
    write: (p, s) => {
      fs.mkdirSync(path.dirname(path.join(vault, p)), { recursive: true });
      fs.writeFileSync(path.join(vault, p), s);
    },
    rm: (p) => fs.rmSync(path.join(vault, p)),
    files: () => listLocal(vault),
  };
}

/** { relative path: sha1 } of every file outside dot-folders. */
function listLocal(root) {
  const out = {};
  const walk = (dir, rel) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      if (e.name.startsWith(".")) continue;
      const r = rel ? `${rel}/${e.name}` : e.name;
      if (e.isDirectory()) walk(path.join(dir, e.name), r);
      else out[r] = crypto.createHash("sha1").update(fs.readFileSync(path.join(dir, e.name))).digest("hex");
    }
  };
  walk(root, "");
  return out;
}

/**
 * Same for a folder on the device (`sh` = devSh for shared storage, runAs for
 * app storage). Like listLocal it skips every hidden name: Cairn ignores them,
 * and a kill during an app-storage save leaves a hidden temp file behind
 * (checked separately in adv_lifecycle.test.mjs, FINDING-177).
 */
function listDevice(sh, root) {
  const out = {};
  const txt = sh(`cd ${q(root)} && find . -type f -not -path '*/.*' -exec sha1sum {} + 2>/dev/null || true`);
  for (const line of txt.split("\n").filter(Boolean)) {
    const m = line.match(/^([0-9a-f]{40})\s+\.\/(.*)$/);
    if (m) out[m[2]] = m[1];
  }
  return out;
}

async function syncPhone() {
  const st = await invoke("sync_now");
  return st;
}

/** Sync phone and desktop back and forth until both report nothing to do (or give up). */
async function converge(desk) {
  let last;
  for (let i = 0; i < 4; i++) {
    const p = await syncPhone();
    const r = desk.sync();
    last = { phone: `${p.state} pulled ${p.lastPulled} pushed ${p.lastPushed} ${p.lastError ?? ""}`, desktop: r };
    if (p.state === "idle" && p.lastPulled + p.lastPushed === 0 && (r.pulled ?? 0) + (r.pushed ?? 0) === 0) break;
  }
  return last;
}

/**
 * Start a sync, type into an open note while it runs, press Home and let
 * Android kill the background process; then restart, sync both devices and
 * compare every file. `sh` reads the vault folder on the device.
 */
async function backgroundDuringSync({ vaultId, folder, sh, notes, openVault }) {
  await setupSync(d, { port, token: TOKEN, vaultId, device: "phone", pass: PASS });
  await closeSettings(d);
  const desk = desktop(vaultId);
  for (let i = 0; i < notes; i++) desk.write(`Desk/D${i}.md`, `# Desk ${i}\n${"desktop line\n".repeat(40)}`);
  const r = desk.sync();
  assert.ok(r.pushed >= notes, JSON.stringify(r));
  await d.openNote("Phone.md", { contains: "phone base" });
  await d.append("typed before the sync\n");
  await sleep(900); // autosave
  void d.eval(`window.__TAURI_INTERNALS__.invoke('sync_now').catch(() => {})`).catch(() => {});
  await sleep(300);
  await d.append("typed during the sync\n");
  key("KEYCODE_HOME"); // flushes the pending edit
  await sleep(400);
  const midway = Object.keys(listDevice(sh, folder)).filter((p) => p.startsWith("Desk/")).length;
  if (!(await amKill())) {
    console.log("am kill did not stop the process (still busy); using SIGKILL like the low-memory killer");
    kill9();
  }
  await eventually(() => !appPid(), { timeout: 10000, message: "background process killed" });
  console.log(`desktop notes on the phone when the process was killed: ${midway} of ${notes}`);
  assert.match(sh(`cat ${q(folder + "/Phone.md")}`), /typed during the sync/, "Home saved the edit typed during the sync");
  await d.launch();
  await openVault();
  const last = await converge(desk);
  console.log("last round:", JSON.stringify(last));
  const phone = listDevice(sh, folder);
  const deskFiles = desk.files();
  assert.deepEqual(Object.keys(phone).sort(), Object.keys(deskFiles).sort(), "same files on both devices");
  for (const [p, h] of Object.entries(deskFiles)) assert.equal(phone[p], h, `${p} has the same content on both devices`);
  assert.match(fs.readFileSync(path.join(desk.vault, "Phone.md"), "utf8"), /typed before the sync\ntyped during the sync/);
  const conflicts = Object.keys(deskFiles).filter((p) => /conflict/.test(p));
  for (const c of conflicts) {
    const orig = c.replace(/ \(conflict [^)]*\)/, "");
    const size = (f) => (fs.existsSync(path.join(desk.vault, f)) ? fs.statSync(path.join(desk.vault, f)).size : -1);
    console.log(`conflict copy ${JSON.stringify(c)} (${size(c)} bytes) next to ${JSON.stringify(orig)} (${size(orig)} bytes); expected ${Buffer.byteLength(`# Desk 0\n${"desktop line\n".repeat(40)}`)}-ish bytes each`);
  }
  // Every desktop note must still have its full text under its own name.
  for (let i = 0; i < notes; i++) {
    const want = `# Desk ${i}\n${"desktop line\n".repeat(40)}`;
    assert.equal(fs.readFileSync(path.join(desk.vault, `Desk/D${i}.md`), "utf8"), want, `Desk/D${i}.md kept its text on the desktop`);
  }
  assert.equal(conflicts.length, 0, `no conflict copies: ${JSON.stringify(conflicts)}`);
  return midway;
}

// ---------------------------------------------------------------- app storage

test("app storage: Home, then a background kill, in the middle of a sync; the devices converge and nothing is lost", async () => {
  const vaultDir = await ensureAppVault(d, "SyncBg");
  writeAppFile(`${vaultDir}/Phone.md`, "phone base\n");
  await rescan(d);
  await backgroundDuringSync({
    vaultId: "bg-app", folder: vaultDir, sh: runAs, notes: 1500,
    openVault: () => d.waitFor(`!!document.querySelector('[data-testid=mobile-files]')`, 20000),
  });
});

// ---------------------------------------------------------------- SAF

/** Show the SAF vault with exactly `files` and sync turned off. */
async function safFresh(files) {
  await ensureSafVault(d, LABEL);
  if ((await invoke("sync_status")).configured) await invoke("sync_disconnect");
  await reopenSaf(d, LABEL, F, files);
}

test("SAF: Home, then a background kill, in the middle of a sync, again and again; no note loses its name or its text", async () => {
  await safFresh({ "Phone.md": "phone base\n" });
  await setupSync(d, { port, token: TOKEN, vaultId: "bg-saf", device: "phone", pass: PASS });
  await closeSettings(d);
  const desk = desktop("bg-saf");
  const N = 300;
  const text = (i) => `# Desk ${i}\n${"desktop line\n".repeat(40)}`;
  for (let i = 0; i < N; i++) desk.write(`Desk/D${i}.md`, text(i));
  desk.sync();
  // Android kills a backgrounded app whenever it needs memory; the next start resumes the sync.
  const kills = [];
  for (let k = 0; k < 12; k++) {
    if (Object.keys(listDevice(devSh, F)).filter((p) => p.startsWith("Desk/")).length >= N) break;
    if (!appPid()) {
      await d.launch();
      if (await d.isWelcome()) await d.openRecent(LABEL); // SAF vaults are not reopened at start (FINDING-029)
    }
    void d.eval(`window.__TAURI_INTERNALS__.invoke('sync_now').catch(() => {})`).catch(() => {});
    await sleep(1000 + Math.floor(Math.random() * 1500));
    key("KEYCODE_HOME");
    await sleep(150);
    kill9(); // what the low-memory killer does
    await eventually(() => !appPid(), { timeout: 10000, message: "process killed" });
    const empty = devSh(`cd ${F} && find . -type f -size 0 -not -path './.*' 2>/dev/null || true`).trim();
    kills.push({ notes: Object.keys(listDevice(devSh, F)).filter((p) => p.startsWith("Desk/")).length, empty: empty ? empty.split("\n") : [] });
  }
  console.log("after each kill: notes on the phone, empty files:", JSON.stringify(kills));
  await d.launch();
  if (await d.isWelcome()) await d.openRecent(LABEL);
  const last = await converge(desk);
  console.log("last round:", JSON.stringify(last));
  const deskFiles = desk.files();
  const conflicts = Object.keys(deskFiles).filter((p) => /conflict/.test(p));
  const size = (f) => (fs.existsSync(path.join(desk.vault, f)) ? fs.statSync(path.join(desk.vault, f)).size : -1);
  for (const c of conflicts) console.log(`desktop: ${JSON.stringify(c)} ${size(c)} bytes, next to ${JSON.stringify(c.replace(/ \(conflict [^)]*\)/, ""))} ${size(c.replace(/ \(conflict [^)]*\)/, ""))} bytes`);
  const phone = listDevice(devSh, F);
  assert.deepEqual(Object.keys(phone).sort(), Object.keys(deskFiles).sort(), "same files on both devices");
  const broken = [];
  for (let i = 0; i < N; i++) {
    const f = path.join(desk.vault, `Desk/D${i}.md`);
    if (!fs.existsSync(f) || fs.readFileSync(f, "utf8") !== text(i)) broken.push(`Desk/D${i}.md (${size(`Desk/D${i}.md`)} bytes)`);
  }
  assert.deepEqual(broken, [], "every desktop note keeps its name and its text on the desktop");
  assert.deepEqual(conflicts, [], "no conflict copies");
  // New notes are written to a hidden temp and renamed into place (FINDING-032);
  // one left by a kill is reused when the sync writes that note again.
  const temps = devSh(`cd ${F} && find . -name '*.cairn-tmp*' 2>/dev/null || true`).trim();
  assert.equal(temps, "", "no temp files are left once the devices have converged");
});

test("SAF: a note deleted on another device a second time (its name is already in .trash) does not stop sync", async () => {
  await safFresh({ "Keep.md": "keep\n" });
  await setupSync(d, { port, token: TOKEN, vaultId: "trash-saf", device: "phone", pass: PASS });
  await closeSettings(d);
  const desk = desktop("trash-saf");
  desk.sync();
  for (const round of ["first", "second"]) {
    desk.write("Untitled.md", `${round} version\n`);
    desk.sync();
    let st = await syncPhone();
    assert.equal(st.state, "idle", `phone got the ${round} Untitled.md: ${st.lastError}`);
    assert.ok(ls(F).includes("Untitled.md"));
    desk.rm("Untitled.md");
    desk.sync();
    st = await syncPhone();
    console.log(`after the ${round} remote delete: state ${st.state}, error ${st.lastError}; root ${JSON.stringify(ls(F))}, .trash ${JSON.stringify(ls(`${F}/.trash`))}`);
  }
  // Is sync still working? A phone edit should reach the desktop.
  devSh(`printf 'keep\\nedited on the phone\\n' > ${F}/Keep.md`);
  await rescan(d);
  const st = await syncPhone();
  desk.sync();
  const deskKeep = fs.readFileSync(path.join(desk.vault, "Keep.md"), "utf8");
  console.log(`after a phone edit: phone state ${st.state} (${st.lastError}); desktop Keep.md ${JSON.stringify(deskKeep)}`);
  await d.click("[data-testid=open-settings]").catch(() => {});
  await d.click("[data-testid=settings-sync]").catch(() => {});
  await sleep(500);
  await d.shot("adv-sync-trash-stuck.png");
  await closeSettings(d);
  assert.ok(!ls(F).includes("Untitled.md"), "the second remote delete removed Untitled.md from the phone");
  assert.equal(st.state, "idle", `sync should keep working, got: ${st.lastError}`);
  assert.match(deskKeep, /edited on the phone/, "the phone's later edit reached the desktop");
});

/** Sync desktop notes with `names` to the SAF vault and back; report what each side ends up with. */
async function nameRoundTrip(vaultId, notes) {
  await safFresh({ "Seed.md": "seed\n" });
  await setupSync(d, { port, token: TOKEN, vaultId, device: "phone", pass: PASS });
  await closeSettings(d);
  const desk = desktop(vaultId);
  for (const [name, text] of Object.entries(notes)) desk.write(name, text);
  desk.sync();
  const first = await syncPhone();
  const phoneFiles = ls(F);
  const second = await syncPhone(); // the phone rescans and pushes what it sees
  const r = desk.sync();
  const deskFiles = fs.readdirSync(desk.vault).filter((f) => !f.startsWith(".")).sort();
  console.log(`phone: sync ${first.state} (${first.lastError ?? "no error"}), files ${JSON.stringify(phoneFiles)}, not synced ${JSON.stringify(first.skipped)}; second phone sync pushed ${second.lastPushed}, not synced ${JSON.stringify(second.skipped)}; desktop then pulled ${r.pulled}, files ${JSON.stringify(deskFiles)}`);
  return { first, second, phoneFiles, deskFiles, desk };
}

// FINDING-172, by design: the phone's storage cannot hold both
// names, so it keeps the first and refuses the second, which is listed under
// "files not synced"; the pull goes on, and no device renames either note.
test("SAF: of two desktop notes whose names differ only in case, the phone stores one and lists the other under files not synced; the desktop keeps both", async () => {
  const notes = { "Notes.md": "UPPER case note\n", "notes.md": "lower case note\n" };
  const { first, second, phoneFiles, deskFiles } = await nameRoundTrip("case-saf", notes);
  assert.deepEqual(deskFiles, ["Notes.md", "Seed.md", "notes.md"], "the phone must not rename or delete the desktop's notes");
  const kept = phoneFiles.filter((f) => !f.startsWith(".") && f !== "Seed.md");
  assert.equal(kept.length, 1, `the phone keeps one of the two, under its own name: ${JSON.stringify(phoneFiles)}`);
  assert.ok(kept[0] in notes, `the phone stores no other name: ${JSON.stringify(phoneFiles)}`);
  assert.equal(devSh(`cat ${q(`${F}/${kept[0]}`)}`), notes[kept[0]], `${kept[0]} on the phone has the desktop's text for that name`);
  const other = Object.keys(notes).find((n) => n !== kept[0]);
  for (const [n, st] of [["first", first], ["second", second]]) {
    assert.equal(st.state, "idle", `${n} phone sync: a refused name must not stop the sync (${st.lastError})`);
    const listed = (st.skipped ?? []).find((x) => x.path === other);
    assert.match(listed?.reason ?? "", /differ only in case/, `${n} phone sync reports ${other} under files not synced: ${JSON.stringify(st.skipped)}`);
  }
});

// FINDING-172, by design: the phone keeps refusing names its shared
// storage cannot hold (no name mapping), and lists each one under "files not
// synced"; the pull goes on, and the desktop keeps the notes under their names.
test("SAF: a desktop note whose name has characters FAT forbids (? \" :) is not stored on the phone, is listed under files not synced, and keeps its name on the desktop", async () => {
  const names = ["What?.md", "Say \"hi\".md"];
  const { first, second, phoneFiles, deskFiles } = await nameRoundTrip("fat-saf", { [names[0]]: "question\n", [names[1]]: "quote\n" });
  assert.deepEqual(deskFiles, ["Say \"hi\".md", "Seed.md", "What?.md"], "the phone must not rename or delete the desktop's notes");
  assert.deepEqual(phoneFiles.filter((f) => !f.startsWith(".")), ["Seed.md"], "the phone keeps no copy under another name");
  for (const [n, st] of [["first", first], ["second", second]]) {
    assert.equal(st.state, "idle", `${n} phone sync: a refused name must not stop the sync (${st.lastError})`);
    const listed = Object.fromEntries((st.skipped ?? []).map((s) => [s.path, s.reason]));
    for (const name of names) assert.match(listed[name] ?? "", /not allowed on this storage/, `${n} phone sync lists ${name} under files not synced with the reason: ${JSON.stringify(st.skipped)}`);
  }
});

/** Files outside dot-folders in the SAF vault folder, sorted. */
const phoneNames = () => ls(F).filter((f) => !f.startsWith(".")).sort();

/** Files directly in the desktop's vault folder, sorted (none of these tests use subfolders). */
const deskNames = (desk) => fs.readdirSync(desk.vault).filter((f) => !f.startsWith(".")).sort();

// FINDING-172, case 1 (by design): the phone holds back a desktop rename it
// cannot store (a name that differs only in case from another note on the
// phone) also when the note is edited on the phone. It must not push the old
// name with the edit, which would undo the rename on every device.
test("SAF: an edit on the phone does not undo a desktop rename to a name that differs only in case from another note", async () => {
  await safFresh({ "Seed.md": "seed\n" });
  await setupSync(d, { port, token: TOKEN, vaultId: "held-saf", device: "phone", pass: PASS });
  await closeSettings(d);
  const desk = desktop("held-saf");
  desk.write("Notes.md", "upper\n");
  desk.write("draft.md", "line 1\n");
  desk.sync();
  await syncPhone();
  assert.deepEqual(phoneNames(), ["Notes.md", "Seed.md", "draft.md"], "the phone has the desktop's notes");
  fs.renameSync(path.join(desk.vault, "draft.md"), path.join(desk.vault, "notes.md"));
  desk.sync();
  const held = await syncPhone();
  devSh(`printf 'line 1\\nphone line\\n' > ${q(`${F}/draft.md`)}`);
  await rescan(d);
  const first = await syncPhone();
  const second = await syncPhone();
  desk.sync();
  console.log(`held rename: phone ${JSON.stringify(phoneNames())}, not synced ${JSON.stringify(second.skipped)}; desktop ${JSON.stringify(deskNames(desk))}`);
  assert.deepEqual(deskNames(desk), ["Notes.md", "Seed.md", "notes.md"], "the desktop keeps its rename");
  assert.equal(fs.readFileSync(path.join(desk.vault, "notes.md"), "utf8"), "line 1\n");
  assert.deepEqual(phoneNames(), ["Notes.md", "Seed.md", "draft.md"], "the phone keeps the note under its old name");
  assert.equal(devSh(`cat ${q(`${F}/draft.md`)}`), "line 1\nphone line\n", "the edit stays on the phone");
  for (const [n, st] of [["held", held], ["first", first], ["second", second]]) {
    assert.equal(st.state, "idle", `${n} phone sync: ${st.lastError}`);
    const listed = (st.skipped ?? []).find((x) => x.path === "notes.md");
    assert.match(listed?.reason ?? "", /differ only in case/, `${n} phone sync lists notes.md under files not synced: ${JSON.stringify(st.skipped)}`);
  }
});

// FINDING-172, case 2 (by design): of two desktop notes whose names differ
// only in case, the phone stores one. Deleting that one on the phone deletes
// it on the desktop too, and the phone then stores the other. A delete that
// stayed on the phone would take the other note instead.
test("SAF: deleting on the phone the one it stores of two notes whose names differ only in case deletes it on the desktop too", async () => {
  const notes = { "Notes.md": "UPPER case note\n", "notes.md": "lower case note\n" };
  const { phoneFiles, desk } = await nameRoundTrip("twin-del-saf", notes);
  const kept = phoneFiles.find((f) => f in notes);
  const other = Object.keys(notes).find((n) => n !== kept);
  await invoke("delete_entry", { path: kept });
  const first = await syncPhone();
  const second = await syncPhone();
  desk.sync();
  const trash = fs.existsSync(path.join(desk.vault, ".trash")) ? fs.readdirSync(path.join(desk.vault, ".trash")) : [];
  console.log(`deleted ${kept} on the phone: phone ${JSON.stringify(phoneNames())}, not synced ${JSON.stringify(second.skipped)}; desktop ${JSON.stringify(deskNames(desk))}, desktop .trash ${JSON.stringify(trash)}`);
  assert.deepEqual(deskNames(desk), ["Seed.md", other].sort(), "the delete reached the desktop");
  assert.ok(trash.includes(kept), `the desktop moved ${kept} to its trash`);
  assert.deepEqual(phoneNames(), ["Seed.md", other].sort(), "the phone stores the other note");
  assert.equal(devSh(`cat ${q(`${F}/${other}`)}`), notes[other], `${other} on the phone has its text`);
  for (const [n, st] of [["first", first], ["second", second]]) {
    assert.equal(st.state, "idle", `${n} phone sync: ${st.lastError}`);
    assert.deepEqual(st.skipped ?? [], [], `${n} phone sync leaves nothing out`);
  }
});

// FINDING-172, case 3 (by design): a note the phone already has under another
// spelling, with the text of a desktop note, is that note and takes the
// desktop's name on the phone. The phone must not push its own spelling, which
// would rename the desktop's note.
test("SAF: a note the phone already has under another spelling takes the desktop's name instead of renaming the desktop's note", async () => {
  const desk = desktop("spell-saf");
  desk.write("Notes.md", "UPPER case note\n");
  desk.write("notes.md", "lower case note\n");
  desk.sync();
  await safFresh({ "Seed.md": "seed\n", "NOTES.md": "UPPER case note\n" });
  await setupSync(d, { port, token: TOKEN, vaultId: "spell-saf", device: "phone", pass: PASS });
  await closeSettings(d);
  const st = await syncPhone();
  desk.sync();
  console.log(`third spelling: phone ${JSON.stringify(phoneNames())}, not synced ${JSON.stringify(st.skipped)}; desktop ${JSON.stringify(deskNames(desk))}`);
  assert.deepEqual(deskNames(desk), ["Notes.md", "Seed.md", "notes.md"], "the desktop keeps its names");
  assert.equal(fs.readFileSync(path.join(desk.vault, "Notes.md"), "utf8"), "UPPER case note\n");
  assert.deepEqual(phoneNames(), ["Notes.md", "Seed.md"], "the phone's copy takes the desktop's spelling");
  assert.equal(st.state, "idle", st.lastError);
  const listed = (st.skipped ?? []).find((x) => x.path === "notes.md");
  assert.match(listed?.reason ?? "", /differ only in case/, `notes.md is listed under files not synced: ${JSON.stringify(st.skipped)}`);
});

test("sync setup: a vault name that differs only in case from the existing one does not silently start a second, empty vault", async () => {
  const desk = desktop("notes");
  desk.write("From desktop.md", "written on the desktop\n");
  desk.sync();
  await ensureAppVault(d, "SyncCase");
  await fillSyncForm(d, { port, token: TOKEN, vaultId: "Notes", device: "phone", pass: PASS });
  // Setup asks before it creates a vault the server does not have.
  await d.waitFor(`!!document.querySelector('[data-testid=dialog-ok]') || !!document.querySelector('[data-testid=sync-error]') || !!document.querySelector('[data-testid=sync-state]')`, 60000);
  const question = await d.eval(`document.querySelector('[data-testid=dialog-ok]')?.closest('[role=dialog]').querySelector('p')?.textContent.trim() ?? null`);
  // The user says no, to correct the name.
  if (question) await d.eval(`(() => { const ok = document.querySelector('[data-testid=dialog-ok]'); [...ok.parentElement.querySelectorAll('button')].find((b) => b !== ok).click(); })()`);
  await sleep(1000);
  const connected = await d.eval(`!!document.querySelector('[data-testid=sync-state]')`);
  const vaultField = await d.eval(`document.querySelector('[data-testid=sync-vault]')?.value ?? null`);
  const vaults = await Promise.all(["notes", "Notes"].map(async (v) => [v, (await fetch(`http://127.0.0.1:${port}${API}/vaults/${v}`, { headers: { authorization: `Bearer ${TOKEN}` } })).status]));
  console.log(`phone set up with "Notes": asked ${JSON.stringify(question)}, connected ${connected}, vault field ${JSON.stringify(vaultField)}; server GET /vaults/<id>: ${JSON.stringify(vaults)}`);
  await closeSettings(d);
  if (connected) await invoke("sync_disconnect");
  assert.equal(question, "There's no vault called Notes on this server. Create it?", "setup should say the vault does not exist yet");
  // Saying no creates nothing and leaves the name in the form to correct.
  assert.deepEqual({ connected, vaultField, vaults }, { connected: false, vaultField: "Notes", vaults: [["notes", 200], ["Notes", 404]] });
});

test("SAF: renaming the shared folder in a file manager does not delete every note on the other devices", async () => {
  await safFresh({ "Alpha.md": "alpha\n", "Beta.md": "beta\n", "Sub/Gamma.md": "gamma\n" });
  await setupSync(d, { port, token: TOKEN, vaultId: "moved-saf", device: "phone", pass: PASS });
  await closeSettings(d);
  const desk = desktop("moved-saf");
  await converge(desk);
  const before = Object.keys(desk.files()).sort();
  assert.deepEqual(before, ["Alpha.md", "Beta.md", "Sub/Gamma.md"], "desktop has the phone's notes");
  // The user tidies up in the Files app: the vault folder gets a new name.
  devSh(`rm -rf ${F}-moved && mv ${F} ${F}-moved`);
  let st;
  let list;
  try {
    list = await invoke("list_entries").catch((e) => `error: ${JSON.stringify(e)}`);
    st = await syncPhone();
    desk.sync();
  } finally {
    devSh(`rm -rf ${F} && mv ${F}-moved ${F}`);
  }
  const after = Object.keys(desk.files()).sort();
  const trash = fs.existsSync(path.join(desk.vault, ".trash")) ? fs.readdirSync(path.join(desk.vault, ".trash")) : [];
  console.log(`folder renamed on the phone: app lists ${typeof list === "string" ? list : list.length + " entries"}; phone sync ${st.state} pushed ${st.lastPushed} (${st.lastError ?? "no error"}); desktop files ${JSON.stringify(after)}, desktop .trash ${JSON.stringify(trash)}`);
  await d.shot("adv-sync-folder-moved.png");
  // Put the desktop back so later runs start clean.
  await invoke("sync_disconnect").catch(() => {});
  assert.deepEqual(after, before, "the desktop must keep every note when the phone's folder merely moved");
  assert.notEqual(st.state, "idle", "the phone should report that the vault folder is gone");
});
