// Adversarial tests: path handling of SafFs / SafPlugin.kt in a
// folder picked with the system picker (Storage Access Framework): odd names,
// renames and moves onto existing names, folders replaced by files, deep
// nesting, the path-to-document-id cache, and names arriving through sync.
//
//   . scripts/android-env.sh
//   node --test --test-concurrency=1 e2e/android/adv_saf_paths.test.mjs
//
// Needs one emulator in `adb devices` with the debug APK installed (installed
// if missing), the prebuilt target/debug/cairn-server and
// target/debug/examples/sync_dir. Clears the app's data and uses
// /sdcard/Documents/AdvSafPaths on the device. Most checks call the app's own
// Tauri commands (the same ones the UI calls) through the WebView DevTools
// protocol, so they exercise SafFs directly.
// Shared storage on the emulator is case-insensitive and refuses the FAT
// characters " * : < > ? \ | in names, like most phones.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import {
  Device, adb, devSh, eventually, APK, PKG, q, ls, fillSharedFolder, startServer, syncDir, setupSync, closeSettings,
  readSettings, restoreSettings, sleep,
} from "./adv_helpers.mjs";

const LABEL = "AdvSafPaths";
const F = `/sdcard/Documents/${LABEL}`;
const TOKEN = "adv-saf-paths-token-0123456789abcdef";
const PASS = "adv saf paths passphrase";
const d = new Device();
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "cairn-adv-af-"));
let settings;
let server;
let port;

before(async () => {
  settings = readSettings();
  if (!devSh(`pm list packages ${PKG}`).includes(PKG)) adb("install", "-r", APK);
  ({ proc: server, port } = startServer(path.join(tmp, "server"), TOKEN));
  await eventually(async () => (await fetch(`http://127.0.0.1:${port}/health`)).ok, { message: "server up" });
  fillSharedFolder(F, { "Seed.md": "seed\n" });
  await d.fresh();
  await d.pickSafFolder(LABEL);
});

after(async () => {
  restoreSettings(settings);
  await d.shot("../AF/adv-saf-paths-final.png").catch(() => {});
  d.close();
  server?.kill();
  try {
    devSh(`rm -rf ${F}`);
  } catch {}
  fs.rmSync(tmp, { recursive: true, force: true });
  adb("forward", "--remove-all");
  setTimeout(() => process.exit(), 1500).unref();
});

/** Call a Tauri command; errors come back as { ERR: <CmdError> } instead of throwing. */
const invoke = (cmd, args = {}) =>
  d.eval(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)}).catch((e) => ({ ERR: e }))`);
const paths = async () => (await invoke("list_entries")).map((e) => e.path).sort();
/** Every file and folder under F (relative, sorted), hidden ones included. */
const tree = () =>
  devSh(`cd ${q(F)} && find . -mindepth 1 2>/dev/null || true`)
    .split("\n")
    .filter(Boolean)
    .map((p) => p.replace(/^\.\//, ""))
    .sort();
const cat = (p) => devSh(`cat ${q(`${F}/${p}`)} 2>/dev/null || echo '<missing>'`);

/** Disconnect sync if set up, refill the folder (device shell) and rescan so the index matches it. */
async function reset(files = {}, shell = "") {
  await d.attach();
  if ((await invoke("sync_status")).configured) await invoke("sync_disconnect");
  fillSharedFolder(F, files);
  if (shell) devSh(`cd ${q(F)} && ${shell}`);
  await invoke("rescan");
}

// ---------------------------------------------------------------------------
// Names and contents made by other apps
// ---------------------------------------------------------------------------

test("SAF: names with # [ ] ^, leading/trailing spaces and trailing or repeated dots made by another app can be read, saved, renamed and deleted", async () => {
  const names = ["h#[x]^.md", " lead.md", "trail .md", "dot.md.", "dots...md", "empty.md"];
  await reset({}, `for n in ${names.slice(0, 5).map(q).join(" ")}; do printf 'text of %s' "$n" > "$n"; done; printf '' > empty.md`);
  assert.deepEqual(await paths(), [...names].sort());
  for (const n of names.filter((n) => n.endsWith(".md"))) {
    const r = await invoke("read_note", { path: n });
    assert.equal(r.ERR, undefined, `${n}: ${JSON.stringify(r.ERR)}`);
    assert.equal(r.content, n === "empty.md" ? "" : `text of ${n}`);
    const w = await invoke("write_note", { path: n, content: r.content + "+saved", baseHash: r.hash });
    assert.equal(w.ERR, undefined, `${n}: ${JSON.stringify(w.ERR)}`);
    assert.equal(cat(n), r.content + "+saved", `${n} saved in place`);
  }
  const ren = await invoke("rename_entry", { from: " lead.md", to: "lead.md" });
  assert.equal(ren.ERR, undefined, JSON.stringify(ren.ERR));
  const del = await invoke("delete_entry", { path: "trail .md" });
  assert.equal(del.ERR, undefined, JSON.stringify(del.ERR));
  assert.deepEqual(tree(), [".trash", ".trash/trail .md", "dot.md.", "dots...md", "empty.md", "h#[x]^.md", "lead.md"]);
  assert.deepEqual(await paths(), ["dot.md.", "dots...md", "empty.md", "h#[x]^.md", "lead.md"]);
});

test("SAF: a note that is not UTF-8 is refused with an error and left untouched", async () => {
  await reset({}, `printf '\\377\\376 latin1 \\351t\\351' > latin.md`);
  const before = devSh(`sha1sum ${q(F + "/latin.md")}`).split(" ")[0];
  const r = await invoke("read_note", { path: "latin.md" });
  assert.match(JSON.stringify(r.ERR ?? null), /not valid UTF-8/);
  assert.equal(devSh(`sha1sum ${q(F + "/latin.md")}`).split(" ")[0], before);
});

test("SAF: the shared folder refuses FAT characters in names, so another app cannot create them (precondition for the sync tests)", async () => {
  await reset();
  const out = devSh(`cd ${q(F)} && for n in 'a:b.md' 'q?.md' 'b\\s.md'; do (printf x > "$n") 2>/dev/null && echo "made $n" || echo "refused $n"; done`);
  assert.match(out, /refused a:b\.md/);
  assert.match(out, /refused q\?\.md/);
});

// FINDING-172, by design: a name the shared storage cannot hold is
// refused (no name mapping) and listed under "files not synced". The app does
// not create such a name either (write_note checks the names of new notes since
// FINDING-140); a pull goes through the vault's write_file, which leaves the
// check to the storage.
test("SAF + sync: a desktop note named 'Plan 10:30.md', which the storage cannot hold, is refused on the phone with no copy under another name and listed under files not synced; the app cannot create that name either", async () => {
  const name = "x/Plan 10:30.md";
  const desk = await syncSetup("af-colon", { "Seed.md": "seed\n" });
  desk.write(name, "plan\n");
  desk.sync();
  const s1 = await invoke("sync_now");
  const s2 = await invoke("sync_now");
  const w = await invoke("write_note", { path: name, content: "typed on the phone", baseHash: null });
  const r = desk.sync();
  const disk = tree();
  console.log(`phone: ${s1.state} (${s1.lastError ?? "no error"}) not synced ${JSON.stringify(s1.skipped)} | ${s2.state} not synced ${JSON.stringify(s2.skipped)}; write_note: ${JSON.stringify(w.ERR ?? "ok")}; phone disk ${JSON.stringify(disk)}; desktop pulled ${r.pulled}, files ${JSON.stringify(Object.keys(desk.files()).sort())}`);
  await invoke("sync_disconnect");
  for (const [n, st] of [["first", s1], ["second", s2]]) {
    assert.equal(st.state, "idle", `${n} phone sync: a refused name must not stop the sync (${st.lastError})`);
    const listed = (st.skipped ?? []).find((x) => x.path === name);
    assert.match(listed?.reason ?? "", /not allowed on this storage/, `${n} phone sync lists ${name} under files not synced: ${JSON.stringify(st.skipped)}`);
  }
  assert.equal(w.ERR?.kind, "invalidName", `write_note must refuse the name: ${JSON.stringify(w)}`);
  assert.deepEqual(disk.filter((p) => p.startsWith("x/")), [], "nothing is stored in x/ under this or another name");
  assert.deepEqual(Object.keys(desk.files()).sort(), ["Seed.md", name], "the desktop keeps the note under its name");
});

// ---------------------------------------------------------------------------
// Renames and moves
// ---------------------------------------------------------------------------

test("SAF: renaming onto an existing name (same case) is refused and changes nothing", async () => {
  await reset({ "a.md": "AAA", "b.md": "BBB", "D/x.md": "dx" });
  const r = await invoke("rename_entry", { from: "a.md", to: "b.md" });
  assert.equal(r.ERR?.kind, "alreadyExists", JSON.stringify(r));
  const r2 = await invoke("rename_entry", { from: "a.md", to: "D/x.md" });
  assert.equal(r2.ERR?.kind, "alreadyExists", JSON.stringify(r2));
  assert.deepEqual(tree(), ["D", "D/x.md", "a.md", "b.md"]);
  assert.equal(cat("b.md"), "BBB");
  assert.equal(cat("D/x.md"), "dx");
});

test("SAF: moving a folder into its own child is refused (also when the case differs) and changes nothing", async () => {
  await reset({ "F/n.md": "n", "F/sub/m.md": "m" });
  const before = tree();
  const r = await invoke("rename_entry", { from: "F", to: "F/sub/F" });
  assert.equal(r.ERR?.kind, "moveIntoSelf", JSON.stringify(r));
  const r2 = await invoke("rename_entry", { from: "F", to: "f/sub/F" });
  assert.ok(r2.ERR, `case-variant move into own child must fail: ${JSON.stringify(r2)}`);
  assert.deepEqual(tree(), before);
  assert.deepEqual(await paths(), ["F", "F/n.md", "F/sub", "F/sub/m.md"]);
});

test("SAF: renaming a note onto a name that differs only in case from another note is refused (the storage is case-insensitive)", async () => {
  await reset({ "a.md": "AAA", "b.md": "BBB" });
  const r = await invoke("rename_entry", { from: "a.md", to: "B.md" });
  const disk = tree();
  const idx = await paths();
  console.log("rename a.md -> B.md:", JSON.stringify(r.ERR ?? r), "| disk:", JSON.stringify(disk), "| index:", JSON.stringify(idx));
  assert.equal(cat("b.md"), "BBB", "the other note is untouched");
  assert.deepEqual(disk, ["a.md", "b.md"], "expected the rename to be refused with nothing changed on disk");
  assert.equal(r.ERR?.kind, "alreadyExists", "expected an 'already exists' error");
});

test("SAF: moving a note into another folder under a new name works when that folder holds a note with the old name", async () => {
  await reset({ "Inbox/Untitled.md": "meeting notes", "Projects/Untitled.md": "another untitled" });
  const r = await invoke("rename_entry", { from: "Inbox/Untitled.md", to: "Projects/Meeting.md" });
  console.log("rename Inbox/Untitled.md -> Projects/Meeting.md:", JSON.stringify(r.ERR ?? r), "| disk:", JSON.stringify(tree()));
  assert.equal(r.ERR, undefined, `move+rename failed: ${JSON.stringify(r.ERR)}`);
  assert.equal(cat("Projects/Meeting.md"), "meeting notes");
  assert.equal(cat("Projects/Untitled.md"), "another untitled");
});

test("SAF: moving a note or folder into another folder under a new name works when both folders hold the old name and the old folder holds the new one, and leaves no stray copies", async () => {
  await reset({ "Inbox/A.md": "inbox a", "Inbox/B.md": "inbox b", "Projects/A.md": "projects a", "Inbox/Sub/n.md": "n", "Inbox/Sub2/o.md": "o", "Projects/Sub/m.md": "m" });
  const r = await invoke("rename_entry", { from: "Inbox/A.md", to: "Projects/B.md" });
  const r2 = await invoke("rename_entry", { from: "Inbox/Sub", to: "Projects/Sub2" });
  console.log("note:", JSON.stringify(r.ERR ?? "ok"), "| folder:", JSON.stringify(r2.ERR ?? "ok"), "| disk:", JSON.stringify(tree()));
  assert.equal(r.ERR, undefined, `note move+rename failed: ${JSON.stringify(r.ERR)}`);
  assert.equal(r2.ERR, undefined, `folder move+rename failed: ${JSON.stringify(r2.ERR)}`);
  const want = ["Inbox", "Inbox/B.md", "Inbox/Sub2", "Inbox/Sub2/o.md", "Projects", "Projects/A.md", "Projects/B.md", "Projects/Sub", "Projects/Sub/m.md", "Projects/Sub2", "Projects/Sub2/n.md"];
  assert.deepEqual(tree(), want);
  assert.deepEqual(await paths(), want);
  for (const [p, text] of Object.entries({ "Projects/B.md": "inbox a", "Projects/A.md": "projects a", "Inbox/B.md": "inbox b", "Projects/Sub2/n.md": "n" })) {
    assert.equal(cat(p), text, `${p} on disk`);
    assert.equal((await invoke("read_note", { path: p })).content, text, `${p} read by the app`);
  }
});

test("SAF: notes saved under names with # [ ] ^ % & ~ ' emoji, leading/trailing spaces, and in folders whose names end in a space or a dot keep their exact names, one file each; the app creates only the names it allows", async () => {
  await reset({ "Seed.md": "seed\n" });
  // write_note refuses these names for a new note, like create_note
  // (FINDING-140). A pull still creates them (write_file: see "SAF + sync:
  // notes with # [ ] ^ ..."), and so can another app; saves over them work.
  const refused = ["h#[x]^.md", " lead.md", "sp /in.md", "dotdir./in.md"];
  for (const n of refused) {
    const w = await invoke("write_note", { path: n, content: "new", baseHash: null });
    assert.equal(w.ERR?.kind, "invalidName", `${n}: ${JSON.stringify(w)}`);
  }
  assert.deepEqual(tree(), ["Seed.md"], "a refused save leaves nothing behind");
  devSh(`cd ${q(F)} && mkdir -p 'sp ' dotdir. && for n in ${refused.map(q).join(" ")}; do printf old > "$n"; done`);
  await invoke("rescan");
  const names = [...refused, "trail .md", "dots...md", "%41 & ~ ' ` = + ; , @ $ !.md", "日本語 ✓ 🙂.md"];
  for (const n of names) {
    for (const content of ["one", "two"]) {
      const w = await invoke("write_note", { path: n, content, baseHash: null });
      assert.equal(w.ERR, undefined, `${n}: ${JSON.stringify(w.ERR)}`);
    }
    assert.equal((await invoke("read_note", { path: n })).content, "two", n);
    assert.equal(cat(n), "two", `${n} on disk`);
  }
  assert.deepEqual(tree(), ["Seed.md", ...names, "sp ", "dotdir."].sort());
});

test("SAF: saving a note whose name is longer than 255 bytes (legal on macOS, e.g. 90 CJK characters) fails cleanly instead of saving under a cut name", async () => {
  await reset({ "Seed.md": "seed\n" });
  const n = "長".repeat(90) + ".md"; // 273 bytes
  const w1 = await invoke("write_note", { path: n, content: "one", baseHash: null });
  const w2 = await invoke("write_note", { path: n, content: "two", baseHash: null });
  const r = await invoke("read_note", { path: n });
  const disk = tree().filter((x) => x !== "Seed.md");
  console.log(`name of ${Buffer.byteLength(n)} bytes: save 1 ${JSON.stringify(w1.ERR ?? "ok")} | save 2 ${JSON.stringify(w2.ERR ?? "ok").slice(0, 160)}... | read ${JSON.stringify(r.ERR ?? r.content).slice(0, 80)} | disk: ${disk.map((x) => `${Buffer.byteLength(x)}-byte name '${x.slice(0, 4)}...${x.slice(-6)}' = ${cat(x)}`)}`);
  if (w1.ERR) {
    assert.deepEqual(disk, [], "a refused save leaves no file behind");
  } else {
    assert.equal(w2.ERR, undefined, "a save that succeeded once keeps working");
    assert.equal(r.ERR, undefined, "a note that was saved can be read back");
  }
});

test("SAF: renaming a folder onto an existing folder or file name is refused and changes nothing", async () => {
  await reset({ "A/a.md": "a", "B/b.md": "b", "C.md": "c" });
  assert.equal((await invoke("rename_entry", { from: "A", to: "B" })).ERR?.kind, "alreadyExists");
  assert.equal((await invoke("rename_entry", { from: "A", to: "C.md" })).ERR?.kind, "alreadyExists");
  assert.equal((await invoke("rename_entry", { from: "C.md", to: "B" })).ERR?.kind, "alreadyExists");
  assert.deepEqual(tree(), ["A", "A/a.md", "B", "B/b.md", "C.md"]);
});

test("SAF: renaming a folder onto a name that differs only in case from another folder is refused", async () => {
  await reset({ "A/a.md": "a", "B/b.md": "b" });
  const r = await invoke("rename_entry", { from: "A", to: "b" });
  const disk = tree();
  const idx = await paths();
  console.log("rename folder A -> b:", JSON.stringify(r.ERR ?? r), "| disk:", JSON.stringify(disk), "| index:", JSON.stringify(idx));
  assert.deepEqual(disk, ["A", "A/a.md", "B", "B/b.md"], "expected the rename to be refused with nothing changed on disk");
  assert.equal(r.ERR?.kind, "alreadyExists");
});

test("SAF: saving or moving a note into a folder whose name differs only in case from an existing folder ('notes/b.md' next to 'Notes/') is refused, so the two are never merged", async () => {
  await reset({ "Notes/a.md": "A", "Notes/same.md": "upper", "x.md": "X" });
  const w1 = await invoke("write_note", { path: "notes/b.md", content: "B", baseHash: null });
  const w2 = await invoke("write_note", { path: "notes/same.md", content: "lower", baseHash: null });
  const r = await invoke("rename_entry", { from: "x.md", to: "notes/x.md" });
  const disk = tree();
  console.log("save notes/b.md, save notes/same.md, move x.md -> notes/x.md:", JSON.stringify([w1.ERR ?? "ok", w2.ERR ?? "ok", r.ERR ?? "ok"]), "| disk:", JSON.stringify(disk));
  assert.deepEqual(disk, ["Notes", "Notes/a.md", "Notes/same.md", "x.md"], "nothing is written or moved into Notes");
  assert.equal(cat("Notes/same.md"), "upper", "the note in Notes is not overwritten");
  for (const e of [w1, w2, r]) assert.equal(e.ERR?.kind, "alreadyExists", JSON.stringify(e));
  await invoke("rescan");
  assert.deepEqual(await paths(), ["Notes", "Notes/a.md", "Notes/same.md", "x.md"]);
});

test("SAF: a move into a folder spelled in another case that then fails (name too long) leaves the folder's name as it was", async () => {
  await reset({ "Notes/a.md": "A", "Notes/b.md": "B" });
  const r = await invoke("rename_entry", { from: "Notes/a.md", to: `notes/${"長".repeat(90)}.md` });
  const disk = tree();
  console.log("move Notes/a.md -> notes/<273-byte name>:", JSON.stringify(r.ERR ?? "ok").slice(0, 160), "| disk:", JSON.stringify(disk).slice(0, 200));
  assert.ok(r.ERR, "the storage cannot hold the name");
  assert.deepEqual(disk, ["Notes", "Notes/a.md", "Notes/b.md"], "the folder keeps its name and its notes");
});

test("SAF: a note open in the app that another app renames by case only follows the rename; the next edit saves into it with no ghost entry", async () => {
  await reset({ "hello.md": "hi there", "Other.md": "o" });
  await d.openNote("hello.md", { contains: "hi there" });
  devSh(`cd ${q(F)} && mv hello.md tmp && mv tmp Hello.md`);
  await invoke("rescan"); // what the 20 s timer and app resume do
  await eventually(() => d.eval(`document.querySelector('.mobile-title')?.textContent.trim() === 'Hello'`), { message: "tab follows the rename" });
  await d.append(" + edit");
  await eventually(() => cat("Hello.md") === "hi there + edit", { message: "edit saved into Hello.md" });
  await invoke("rescan");
  assert.deepEqual(tree(), ["Hello.md", "Other.md"]);
  assert.deepEqual(await paths(), ["Hello.md", "Other.md"]);
  assert.deepEqual(await d.eval(`[...document.querySelectorAll('[data-testid=tree-row]')].map(r => r.dataset.path).sort()`), ["Hello.md", "Other.md"]);
});

// ---------------------------------------------------------------------------
// Things another app changes underneath the app (stale document-id cache)
// ---------------------------------------------------------------------------

test("SAF: a note replaced by a folder of the same name (and a folder by a file) is not written into; the rescan picks up the new kinds", async () => {
  await reset({ "R.md": "orig", "Q/q.md": "q" });
  const r = await invoke("read_note", { path: "R.md" }); // caches R.md's document id
  assert.equal(r.content, "orig");
  assert.equal((await invoke("read_note", { path: "Q/q.md" })).content, "q");
  devSh(`cd ${q(F)} && rm R.md && mkdir R.md && printf inner > R.md/inner.md && rm -r Q && printf notdir > Q`);
  const w1 = await invoke("write_note", { path: "R.md", content: "edited", baseHash: r.hash });
  const w2 = await invoke("write_note", { path: "R.md", content: "edited", baseHash: null });
  const w3 = await invoke("write_note", { path: "Q/q.md", content: "qq", baseHash: null });
  console.log("errors:", JSON.stringify([w1.ERR, w2.ERR, w3.ERR]));
  assert.ok(w1.ERR && w2.ERR && w3.ERR, "every save must fail");
  assert.deepEqual(tree(), ["Q", "R.md", "R.md/inner.md"]);
  assert.equal(cat("R.md/inner.md"), "inner");
  assert.equal(cat("Q"), "notdir");
  await invoke("rescan");
  const kinds = (await invoke("list_entries")).map((e) => `${e.path}:${e.kind}`).sort();
  assert.deepEqual(kinds, ["Q:file", "R.md/inner.md:file", "R.md:dir"]);
});

test("SAF: cached document ids follow renames, deletes and re-creations made by another app", async () => {
  await reset({ "Sub/x.md": "old x", "A.md": "aaa", "B.md": "bbb", "Gone.md": "gone", "Dir/y.md": "y" });
  for (const p of ["Sub/x.md", "A.md", "B.md", "Gone.md", "Dir/y.md"]) assert.ok(!(await invoke("read_note", { path: p })).ERR);
  // Another app: renames a folder and puts a new one with the same name in place,
  // swaps two notes, deletes one, and renames the folder of another.
  devSh(`cd ${q(F)} && mv Sub Old && mkdir Sub && printf 'new x' > Sub/x.md && mv A.md t && mv B.md A.md && mv t B.md && rm Gone.md && mv Dir Dir2`);
  assert.equal((await invoke("read_note", { path: "Sub/x.md" })).content, "new x");
  assert.equal((await invoke("read_note", { path: "A.md" })).content, "bbb");
  assert.equal((await invoke("read_note", { path: "B.md" })).content, "aaa");
  assert.equal((await invoke("read_note", { path: "Gone.md" })).ERR?.kind, "notFound");
  const w = await invoke("write_note", { path: "Dir/y.md", content: "saved after the folder moved", baseHash: null });
  console.log("save into a folder another app renamed:", JSON.stringify(w.ERR ?? w.entry));
  assert.ok(w.ERR, "saving into a folder that no longer exists fails (no silent write elsewhere)");
  assert.equal(cat("Dir2/y.md"), "y");
  assert.ok(!tree().includes("Dir"), "no folder re-created behind the user's back");
});

test("SAF: notes 40 folders deep can be created, listed, read, renamed and deleted", async () => {
  const deep = Array.from({ length: 40 }, (_, i) => `level${i}`).join("/");
  await reset({ "Seed.md": "seed\n" });
  const c = await invoke("create_note", { path: `${deep}/Deep.md`, content: "deep" });
  assert.equal(c.ERR, undefined, JSON.stringify(c.ERR));
  assert.equal(cat(`${deep}/Deep.md`), "deep");
  assert.ok((await paths()).includes(`${deep}/Deep.md`));
  assert.equal((await invoke("read_note", { path: `${deep}/Deep.md` })).content, "deep");
  assert.equal((await invoke("rename_entry", { from: `${deep}/Deep.md`, to: `${deep}/Deeper.md` })).ERR, undefined);
  await invoke("rescan");
  assert.ok((await paths()).includes(`${deep}/Deeper.md`));
  assert.equal((await invoke("delete_entry", { path: "level0" })).ERR, undefined);
  assert.ok(tree().includes(`.trash/${deep}/Deeper.md`));
});

test("SAF: folders deleted by another app while a rescan walks the tree cause no error, and the next rescan matches the disk", async () => {
  const mk = "for d in a b c d e f g h; do mkdir -p $d; for i in $(seq 1 150); do printf $i > $d/n$i.md; done; done";
  const total = 1 + 8 * 151;
  const seen = [];
  // Delete at once, so the deletion overlaps the walk; repeat until a rescan saw only part of it.
  for (let i = 0; i < 4 && !seen.some((s) => s.partial); i++) {
    await reset({ "keep.md": "k" }, mk);
    assert.equal((await paths()).length, total);
    await d.eval(`window.__afScan = null; window.__TAURI_INTERNALS__.invoke('rescan').then((r) => window.__afScan = { changes: r.length }, (e) => window.__afScan = { err: e }); 1`);
    devSh(`cd ${q(F)} && rm -rf a b c d e f g h`);
    const r = await eventually(() => d.eval(`window.__afScan`), { timeout: 60000, message: "rescan finished" });
    const left = (await paths()).length;
    seen.push({ ...r, indexAfter: left, partial: left > 1 && left < total });
    assert.equal(r.err, undefined, JSON.stringify(r.err));
    await invoke("rescan");
    assert.deepEqual(await paths(), ["keep.md"]);
  }
  console.log("rescans overlapping the deletion:", JSON.stringify(seen));
  assert.ok(seen.some((s) => s.partial), "at least one rescan ran while the folders were being deleted");
});

// ---------------------------------------------------------------------------
// Through sync
// ---------------------------------------------------------------------------

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
    mv: (a, b) => {
      fs.mkdirSync(path.dirname(path.join(vault, b)), { recursive: true });
      fs.renameSync(path.join(vault, a), path.join(vault, b));
    },
    files: () => {
      const out = {};
      const walk = (abs, rel) => {
        for (const e of fs.readdirSync(abs, { withFileTypes: true })) {
          if (e.name.startsWith(".")) continue;
          const r = rel ? `${rel}/${e.name}` : e.name;
          if (e.isDirectory()) walk(path.join(abs, e.name), r);
          else out[r] = fs.readFileSync(path.join(abs, e.name), "utf8");
        }
      };
      walk(vault, "");
      return out;
    },
  };
}

async function syncSetup(vaultId, files) {
  await reset(files);
  await setupSync(d, { port, token: TOKEN, vaultId, device: "phone", pass: PASS });
  await closeSettings(d);
  const desk = desktop(vaultId);
  desk.sync();
  return desk;
}

test("SAF + sync: a note moved to another folder and renamed on the desktop, where that folder has a note with the old name, arrives on the phone", async () => {
  const desk = await syncSetup("af-move-rename", { "Inbox/Untitled.md": "meeting notes\n", "Projects/Untitled.md": "another untitled\n" });
  assert.deepEqual(Object.keys(desk.files()).sort(), ["Inbox/Untitled.md", "Projects/Untitled.md"], "desktop got the phone's notes");
  desk.mv("Inbox/Untitled.md", "Projects/Meeting.md");
  desk.sync();
  const st1 = await invoke("sync_now");
  const st2 = await invoke("sync_now");
  // Is sync still working afterwards? A phone edit should reach the desktop.
  devSh(`printf 'edited on the phone\\n' > ${q(F + "/Projects/Untitled.md")}`);
  await invoke("rescan");
  const st3 = await invoke("sync_now");
  desk.sync();
  const deskFiles = desk.files();
  console.log(`phone syncs: ${[st1, st2, st3].map((s) => `${s.state} (${s.lastError ?? "no error"})`).join(" | ")}; phone disk ${JSON.stringify(tree())}; desktop ${JSON.stringify(deskFiles)}`);
  await invoke("sync_disconnect");
  assert.equal(st1.state, "idle", `phone sync failed: ${st1.lastError}`);
  assert.equal(cat("Projects/Meeting.md"), "meeting notes\n");
  assert.equal(deskFiles["Projects/Untitled.md"], "edited on the phone\n", "sync keeps working");
});

test("SAF + sync: a folder renamed on the desktop by changing only its case ('Notes' -> 'notes') keeps its notes on the phone and sync keeps working", async () => {
  const desk = await syncSetup("af-folder-case", { "Notes/one.md": "one\n", "Notes/two.md": "two\n" });
  assert.deepEqual(Object.keys(desk.files()).sort(), ["Notes/one.md", "Notes/two.md"]);
  fs.renameSync(path.join(desk.vault, "Notes"), path.join(desk.vault, "notes"));
  desk.sync();
  const states = [];
  for (let i = 0; i < 3; i++) {
    const s = await invoke("sync_now");
    states.push(`${s.state} (${s.lastError ?? "no error"})`);
  }
  desk.write("notes/one.md", "one, edited on the desktop\n");
  desk.sync();
  const last = await invoke("sync_now");
  states.push(`${last.state} (${last.lastError ?? "no error"})`);
  const phone = tree();
  const deskFiles = Object.keys(desk.files()).sort();
  console.log(`phone syncs: ${states.join(" | ")}; phone disk ${JSON.stringify(phone)}; phone index ${JSON.stringify(await paths())}; desktop ${JSON.stringify(deskFiles)}`);
  await invoke("sync_disconnect");
  assert.deepEqual(deskFiles, ["notes/one.md", "notes/two.md"], "the desktop keeps its names");
  assert.deepEqual(phone.filter((p) => !p.startsWith(".")), ["notes", "notes/one.md", "notes/two.md"], "the phone's folder follows the rename, with no stray folders");
  assert.equal(last.state, "idle", `phone sync failed: ${last.lastError}`);
  assert.equal(cat("notes/one.md"), "one, edited on the desktop\n", "a later desktop edit arrives");
});

test("SAF + sync: desktop folders whose names differ only in case ('Notes' and 'notes') are not merged on the phone, and the desktop keeps both", async () => {
  const desk = await syncSetup("af-folder-case-pair", { "Seed.md": "seed\n" });
  const want = { "Seed.md": "seed\n", "Notes/a.md": "A\n", "Notes/same.md": "upper\n", "notes/b.md": "B\n", "notes/same.md": "lower\n" };
  for (const [p, c] of Object.entries(want)) desk.write(p, c);
  desk.sync();
  const s1 = await invoke("sync_now");
  const s2 = await invoke("sync_now");
  const r = desk.sync();
  const phone = tree().filter((p) => !p.startsWith("."));
  const deskFiles = desk.files();
  console.log(`phone syncs: ${[s1, s2].map((s) => `${s.state} (${s.lastError ?? "no error"})`).join(" | ")}; phone disk ${JSON.stringify(phone)}; desktop pulled ${r.pulled}, files ${JSON.stringify(Object.keys(deskFiles).sort())}`);
  await invoke("sync_disconnect");
  assert.deepEqual(deskFiles, want, "the phone must not move or change the desktop's notes");
  for (const p of phone.filter((p) => p.endsWith(".md"))) assert.equal(cat(p), want[p], `the phone's ${p} holds that note, not one from the other folder`);
});

test("SAF + sync: notes with # [ ] ^ and leading/trailing spaces, an empty note, a note that is not UTF-8 and a binary file arrive byte for byte and keep their names everywhere", async () => {
  const desk = await syncSetup("af-odd", { "Seed.md": "seed\n" });
  const files = { "h#[x]^.md": "hash\n", " lead.md": "lead\n", "trail .md": "trail\n", "Empty.md": "", "sp /in.md": "in\n", "dotdir./in.md": "dotdir\n" };
  for (const [p, c] of Object.entries(files)) desk.write(p, c);
  fs.writeFileSync(path.join(desk.vault, "latin.md"), Buffer.from([0xff, 0xfe, 0x20, 0xe9, 0x74, 0xe9]));
  fs.writeFileSync(path.join(desk.vault, "blob.bin"), Buffer.from([0, 1, 2, 0xff]));
  desk.sync();
  const s1 = await invoke("sync_now");
  const s2 = await invoke("sync_now");
  const r = desk.sync();
  console.log(`phone: ${s1.state} pulled ${s1.lastPulled} pushed ${s1.lastPushed}; again pushed ${s2.lastPushed}; desktop then pulled ${r.pulled}; phone disk ${JSON.stringify(tree())}`);
  await invoke("sync_disconnect");
  assert.equal(s1.state, "idle", s1.lastError);
  for (const [p, c] of Object.entries(files)) assert.equal(cat(p), c, p);
  assert.equal(devSh(`od -An -tx1 ${q(F + "/latin.md")}`).trim(), "ff fe 20 e9 74 e9");
  assert.equal(devSh(`od -An -tx1 ${q(F + "/blob.bin")}`).trim(), "00 01 02 ff");
  assert.equal(s2.lastPushed, 0, "the phone pushes nothing back");
  assert.equal(r.pulled, 0);
  assert.deepEqual(Object.keys(desk.files()).sort(), ["Seed.md", "blob.bin", "latin.md", ...Object.keys(files)].sort());
});

test("SAF + sync: a desktop file whose name ends in a dot ('draft.') keeps its name on the desktop", async () => {
  const desk = await syncSetup("af-trail-dot", { "Seed.md": "seed\n" });
  desk.write("draft.", "draft text\n");
  desk.write("notes.md.", "x\n");
  desk.sync();
  const s1 = await invoke("sync_now");
  const s2 = await invoke("sync_now");
  const r = desk.sync();
  const deskNames = Object.keys(desk.files()).sort();
  console.log(`phone: ${s1.state} pulled ${s1.lastPulled} pushed ${s1.lastPushed}; again pushed ${s2.lastPushed}; desktop then pulled ${r.pulled}; phone disk ${JSON.stringify(tree())}; desktop ${JSON.stringify(deskNames)}`);
  await invoke("sync_disconnect");
  assert.deepEqual(deskNames, ["Seed.md", "draft.", "notes.md."], "the phone must not rename the desktop's files");
});

test("SAF + sync: a desktop file replaced by a folder of the same name arrives on the phone", async () => {
  const desk = await syncSetup("af-file-to-dir", { "Seed.md": "seed\n", "Topic": "a file\n" });
  fs.rmSync(path.join(desk.vault, "Topic"));
  desk.write("Topic/a.md", "now a folder\n");
  desk.sync();
  const s1 = await invoke("sync_now");
  const s2 = await invoke("sync_now");
  console.log(`phone: ${s1.state} (${s1.lastError}) | ${s2.state} (${s2.lastError}); phone disk ${JSON.stringify(tree())}`);
  await invoke("sync_disconnect");
  assert.equal(s1.state, "idle", s1.lastError);
  assert.equal(cat("Topic/a.md"), "now a folder\n");
});

test("SAF + sync: a desktop folder replaced by a file of the same name arrives on the phone", async () => {
  const desk = await syncSetup("af-dir-to-file", { "Seed.md": "seed\n", "Old/x.md": "x\n" });
  fs.rmSync(path.join(desk.vault, "Old"), { recursive: true });
  desk.write("Old", "now a file\n");
  desk.sync();
  const s1 = await invoke("sync_now");
  const s2 = await invoke("sync_now");
  console.log(`phone: ${s1.state} (${s1.lastError}) | ${s2.state} (${s2.lastError}); phone disk ${JSON.stringify(tree())}`);
  await invoke("sync_disconnect");
  assert.equal(s2.state, "idle", s2.lastError);
  assert.equal(cat("Old"), "now a file\n");
});
