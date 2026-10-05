// Reproduction for FINDING-009: edits still inside the 600 ms autosave
// debounce are lost when the window (or the process) goes away.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_03.test.mjs
//
// The control test (wait longer than the debounce, then close) must pass; it
// shows the close mechanism and the typing helper are sound. The todo test
// is the case that still loses the edit.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { Env, APP, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("vdl03");
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

/** PIDs of target/debug/cairn processes started by this Env (matched by its private XDG_CONFIG_HOME). */
function appPids() {
  const want = `XDG_CONFIG_HOME=${path.join(env.tmp, "config")}`;
  const out = [];
  for (const d of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(d)) continue;
    try {
      if (fs.readlinkSync(`/proc/${d}/exe`) !== APP) continue;
      const envs = fs.readFileSync(`/proc/${d}/environ`, "utf8").split("\0");
      if (envs.includes(want)) out.push(Number(d));
    } catch {}
  }
  return out;
}

test("control: closing the window 1.5 s after typing keeps the edit", async () => {
  const { v, app } = await openFresh({ "A.md": "alpha\n" }, "A.md");
  try {
    await app.typeEnd(" last words");
    await sleep(1500);
    await app.closeWindow();
    await sleep(1500);
    assert.deepEqual(appPids(), [], "the app exited");
    assert.equal(v.read("A.md"), "alpha\n last words");
  } finally {
    await app.close();
  }
});

test(
  "closing the window 200 ms after typing keeps the edit",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n" }, "A.md");
    try {
      await app.typeEnd(" last words");
      assert.equal(await app.editorText(), "alpha\n last words", "the editor has the typed text");
      await sleep(200);
      await app.closeWindow();
      await sleep(2000);
      console.log("cairn processes still alive after Close Window:", appPids());
      assert.deepEqual(appPids(), [], "the app exited");
      assert.equal(v.read("A.md"), "alpha\n last words");
    } finally {
      await app.close();
    }
  },
);

/** The app's process and every process it started (bwrap, the WebKit web and network processes). */
function processTree(root) {
  const kids = new Map();
  for (const d of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(d)) continue;
    try {
      const st = fs.readFileSync(`/proc/${d}/stat`, "utf8");
      const ppid = Number(st.slice(st.lastIndexOf(")") + 2).split(" ")[1]);
      kids.set(ppid, [...(kids.get(ppid) ?? []), Number(d)]);
    } catch {}
  }
  const out = [root];
  for (let i = 0; i < out.length; i++) out.push(...(kids.get(out[i]) ?? []));
  return out;
}

test("SIGTERM to the app (kill, killall) 200 ms after typing keeps the edit, and the app exits", async () => {
  const { v, app } = await openFresh({ "A.md": "alpha\n" }, "A.md");
  try {
    const pids = appPids();
    assert.equal(pids.length, 1, `exactly one app process (${pids})`);
    await app.typeEnd(" last words");
    await sleep(200);
    process.kill(pids[0], "SIGTERM");
    await sleep(2000);
    assert.equal(v.read("A.md"), "alpha\n last words");
    assert.deepEqual(appPids(), [], "the app exited");
  } finally {
    await app.close().catch(() => {});
  }
});

test("SIGTERM while a conflict blocks a save: the question is up, and the app still exits within a few seconds", async () => {
  const { v, app } = await openFresh({ "A.md": "alpha\n" }, "A.md");
  try {
    const pids = appPids();
    await app.insertEnd("MINE");
    v.write("A.md", "THEIRS\n");
    await app.waitBanner();
    process.kill(pids[0], "SIGTERM");
    await sleep(1000);
    const asked = await app.exec(`return /Discard unsaved changes to "A"\\?/.test(document.querySelector(".dialog")?.textContent ?? "")`);
    assert.ok(asked, "the close asks about the conflicted tab");
    assert.deepEqual(appPids(), pids, "still running while the question is up");
    await sleep(4000);
    assert.deepEqual(appPids(), [], "exited after the grace period");
    assert.equal(v.read("A.md"), "THEIRS\n");
  } finally {
    await app.close().catch(() => {});
  }
});

// systemd stops an app at logout or shutdown by signalling every process in
// its scope at once (Ctrl+C or a closed terminal signal the whole process
// group the same way). The web view's process dies at once, and the edits
// that still wait for autosave live only there.
test(
  "SIGTERM to every process of the app at once (systemd stopping its scope) keeps the edit",
  { todo: "FINDING-009: the unsaved text lives only in the web view's process, which dies on the same signal" },
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n" }, "A.md");
    try {
      const [pid] = appPids();
      await app.typeEnd(" last words");
      await sleep(200);
      for (const p of processTree(pid)) {
        try {
          process.kill(p, "SIGTERM");
        } catch {}
      }
      await sleep(4500);
      assert.equal(v.read("A.md"), "alpha\n last words");
    } finally {
      await app.close().catch(() => {});
    }
  },
);

test(
  "closing the window while a tab's save is blocked by a conflict asks first (or keeps the edit somewhere)",
  async () => {
    const { v, app } = await openFresh({ "A.md": "alpha\n" }, "A.md");
    try {
      await app.insertEnd("MINE-LONG-EDIT");
      v.write("A.md", "THEIRS\n");
      await app.waitBanner();
      await app.closeWindow();
      await sleep(2000);
      const asked = await app
        .exec(`return /Discard unsaved changes to "A"\\?/.test(document.querySelector(".dialog")?.textContent ?? "")`)
        .catch(() => false);
      const all = [];
      const walk = (d) => {
        for (const e of fs.readdirSync(d, { withFileTypes: true })) {
          const p = path.join(d, e.name);
          if (e.isDirectory()) walk(p);
          else if (e.isFile()) all.push(p);
        }
      };
      walk(v.root);
      const kept = all.filter((p) => fs.readFileSync(p, "utf8").includes("MINE-LONG-EDIT"));
      console.log("A.md now:", JSON.stringify(v.read("A.md")), "files holding the edit:", kept, "asked:", asked);
      assert.ok(asked || kept.length > 0, "no question asked, and the conflicted edit is nowhere in the vault");
    } finally {
      await app.close();
    }
  },
);

// Without both permissions the UI could not take over a close request and
// finish it after saving. Asked about a window that does not
// exist, an allowed command fails with "not found", a denied one with "not
// allowed" (asking about "main" would close the app under test).
test("the UI may close and destroy its own window (it takes over a close request, saves, then finishes it)", async () => {
  const { app } = await openFresh({ "A.md": "alpha\n" }, "A.md");
  try {
    const destroy = await app.invoke("plugin:window|destroy", { label: "no-such-window" });
    const close = await app.invoke("plugin:window|close", { label: "no-such-window" });
    console.log("destroy:", JSON.stringify(destroy), "close:", JSON.stringify(close));
    assert.doesNotMatch(JSON.stringify(destroy), /not allowed/, "destroy is allowed");
    assert.doesNotMatch(JSON.stringify(close), /not allowed/, "close is allowed");
  } finally {
    await app.close();
  }
});
