// Regression tests for FINDING-130: an inotify queue overflow
// (notify Flag::Rescan, an event with no paths) was dropped by
// app/src-tauri/src/watcher.rs, so files whose events were lost during the
// overflow never reached the index until something else triggered a full
// rescan; the watcher now does a full rescan on an overflow. With the defect,
// 32-36 of 113 burst files stayed unindexed 15 s later; one readdir of the
// vault root (file manager, ls, git status) or the app's own
// visibilitychange rescan healed it, because an IN_OPEN on the root folder
// maps to vault path "" and rescan_paths("") does a full rescan. That full
// rescan opens the root again, so it then repeats every ~250 ms forever
// (diagnostics below), which also masks overflow losses for the rest of the
// session.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_12.test.mjs
//      scripts/e2e-headless.sh --test-name-pattern 'burst' e2e/adv_verify_dl_12.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import * as require_cp from "node:child_process";
import { Env, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  // notify logs the overflow event (wd -1) as "unknown descriptor" at debug.
  process.env.RUST_LOG = "info,notify=debug";
  env = await Env.create("v12");
});
after(async () => {
  if (process.env.V12_LOG) fs.writeFileSync(process.env.V12_LOG, env?.log() ?? "");
  await env?.dispose();
});

/** CPU time (utime+stime, clock ticks) of the cairn process serving `vaultRoot`. */
function cairnCpu(vaultRoot) {
  for (const pid of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(pid)) continue;
    try {
      const cmd = fs.readFileSync(`/proc/${pid}/cmdline`, "utf8").split("\0");
      if (cmd[0].endsWith("target/debug/cairn") && cmd.includes(vaultRoot)) {
        const f = fs.readFileSync(`/proc/${pid}/stat`, "utf8").split(") ")[1].split(" ");
        return Number(f[11]) + Number(f[12]);
      }
    } catch {}
  }
  return null;
}

const overflowsSince = (off) => (env.log().slice(off).match(/Q_OVERFLOW/g) ?? []).length;

async function missing(app, v) {
  const idx = new Set((await app.invoke("list_entries")).ok.map((e) => e.path));
  const disk = v.listDisk();
  return { disk: disk.length, idx: idx.size, missing: disk.filter((p) => !idx.has(p)), extra: [...idx].filter((p) => !disk.includes(p)) };
}

// Count IN_OPEN events on a directory itself (opendir/readdir of it) for `secs`.
function dirOpens(dir, secs) {
  const py = String.raw`
import ctypes, os, struct, sys, time, select, json
libc = ctypes.CDLL("libc.so.6", use_errno=True)
d, secs = sys.argv[1], float(sys.argv[2])
fd = libc.inotify_init1(os.O_NONBLOCK)
libc.inotify_add_watch(fd, d.encode(), 0x20)
n = 0; end = time.time() + secs
while time.time() < end:
    r, _, _ = select.select([fd], [], [], max(0, end - time.time()))
    if not r: continue
    buf = os.read(fd, 65536); i = 0
    while i < len(buf):
        w, mask, cookie, ln = struct.unpack_from("iIII", buf, i)
        name = buf[i+16:i+16+ln].rstrip(b"\0"); i += 16 + ln
        if not name: n += 1
print(n)
`;
  return new Promise((resolve) => {
    const { spawn } = require_cp;
    const p = spawn("python3", ["-c", py, dir, String(secs)]);
    let out = "";
    p.stdout.on("data", (d) => (out += d));
    p.on("close", () => resolve(Number(out.trim())));
  });
}

test(
  "burst: files lost in an overflow are indexed without anything listing the vault folder",
  async () => {
    const v = env.vault("v", { "Open.md": "open\n", "log-a.md": "", "log-b.md": "" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("Open.md");
      await sleep(500);
      const off = env.log().length;
      const made = [];
      const fa = fs.openSync(v.p("log-a.md"), "a");
      const fb = fs.openSync(v.p("log-b.md"), "a");
      for (let i = 0; i < 600_000; i++) {
        fs.writeSync(i % 2 ? fa : fb, "x");
        if (i > 30_000 && i % 5_000 === 0) {
          const n = `created-during-burst-${i}.md`;
          fs.writeFileSync(v.p(n), `made at ${i}\n`);
          made.push(n);
        }
      }
      fs.closeSync(fa);
      fs.closeSync(fb);
      // Do not read the vault folder from here on: an opendir of the vault
      // root is itself an inotify event that makes Cairn rescan everything.
      const notIndexed = async () => {
        const idx = new Set((await app.invoke("list_entries")).ok.map((e) => e.path));
        return made.filter((p) => !idx.has(p));
      };
      await sleep(3000);
      const m3 = await notIndexed();
      await sleep(12000);
      const m15 = await notIndexed();
      const ov = overflowsSince(off);
      const sample = m15[0];
      const inTree = sample ? await app.exec(`return !!document.querySelector('[data-testid=tree-row][data-path="${sample}"]')`) : null;
      // Now list the vault folder once, as a file manager, ls or git status would.
      fs.readdirSync(v.root);
      await sleep(1500);
      const afterLs = await notIndexed();
      console.log(
        JSON.stringify({
          overflows: ov,
          created: made.length,
          notIndexedAfter3s: m3.length,
          notIndexedAfter15s: m15.length,
          sampleMissing: sample,
          sampleInTree: inTree,
          notIndexedAfterOneReaddirOfRoot: afterLs.length,
        }),
      );
      assert.ok(ov > 0, "the burst did not overflow the inotify queue on this machine");
      assert.deepEqual(m15, [], `${m15.length} of ${made.length} files still unindexed 15 s after the burst (${ov} Q_OVERFLOW)`);
    } finally {
      await app.close();
    }
  },
);

test("diagnostic: does one readdir of the vault root start a self-sustaining rescan loop?", async () => {
  const v = env.vault("loop", { "Open.md": "open\n", "Sub/A.md": "a\n" });
  for (let i = 0; i < 2000; i++) v.write(`Bulk/f-${i % 40}/n-${i}.md`, `n ${i}\n`);
  const app = await env.launch(v);
  try {
    await app.openFromTree("Open.md");
    await sleep(1500);
    let c0 = cairnCpu(v.root);
    const idle = await dirOpens(v.root, 5);
    const cpuIdle = cairnCpu(v.root) - c0;
    const p = dirOpens(v.root, 5);
    await sleep(200);
    fs.readdirSync(v.root);
    c0 = cairnCpu(v.root);
    const after = await p;
    await sleep(15000);
    const c1 = cairnCpu(v.root);
    const later = await dirOpens(v.root, 5);
    const cpuLater = cairnCpu(v.root) - c1;
    console.log(
      JSON.stringify({ rootOpensIdle5s: idle, cpuTicksIdle5s: cpuIdle, rootOpensFirst5sAfterOneReaddir: after, rootOpens5sWindow20sLater: later, cpuTicks5sWindow20sLater: cpuLater }),
    );
  } finally {
    await app.close();
  }
});

test("diagnostic: the visibilitychange rescan starts the loop too", async () => {
  const v = env.vault("vis", { "Open.md": "open\n" });
  const app = await env.launch(v);
  try {
    await app.openFromTree("Open.md");
    await sleep(1500);
    const idle = await dirOpens(v.root, 3);
    await app.exec(`document.dispatchEvent(new Event('visibilitychange'))`);
    await sleep(3000);
    const after = await dirOpens(v.root, 4);
    console.log(JSON.stringify({ visRootOpensIdle3s: idle, visRootOpens4sWindow3sAfter: after }));
  } finally {
    await app.close();
  }
});

test(
  "burst while the rescan loop is already running (someone listed the vault folder earlier)",
  async () => {
    const v = env.vault("v", { "Open.md": "open\n", "log-a.md": "", "log-b.md": "" });
    const app = await env.launch(v);
    try {
      await app.openFromTree("Open.md");
      await sleep(500);
      fs.readdirSync(v.root);
      await sleep(1000);
      const off = env.log().length;
      const made = [];
      const fa = fs.openSync(v.p("log-a.md"), "a");
      const fb = fs.openSync(v.p("log-b.md"), "a");
      for (let i = 0; i < 600_000; i++) {
        fs.writeSync(i % 2 ? fa : fb, "x");
        if (i > 30_000 && i % 5_000 === 0) {
          const n = `created-during-burst-${i}.md`;
          fs.writeFileSync(v.p(n), `made at ${i}\n`);
          made.push(n);
        }
      }
      fs.closeSync(fa);
      fs.closeSync(fb);
      const notIndexed = async () => {
        const idx = new Set((await app.invoke("list_entries")).ok.map((e) => e.path));
        return made.filter((p) => !idx.has(p));
      };
      await sleep(3000);
      const m3 = await notIndexed();
      await sleep(5000);
      const m8 = await notIndexed();
      console.log(JSON.stringify({ loopRunning: true, overflows: overflowsSince(off), created: made.length, notIndexedAfter3s: m3.length, notIndexedAfter8s: m8.length }));
      assert.deepEqual(m8, []);
    } finally {
      await app.close();
    }
  },
);

function git(cwd, ...args) {
  return execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", ...args], {
    cwd,
    stdio: ["ignore", "pipe", "pipe"],
    maxBuffer: 64 << 20,
  }).toString();
}

test(
  "realistic: git checkout of a branch with 8,000 notes while the app is open",
  async () => {
    const v = env.vault("git", { "Open.md": "open\n" });
    git(v.root, "init", "-q", "-b", "main");
    git(v.root, "add", "-A");
    git(v.root, "commit", "-q", "-m", "main");
    git(v.root, "checkout", "-q", "-b", "big");
    for (let d = 0; d < 80; d++) for (let f = 0; f < 100; f++) v.write(`folder-${d}/note-${f}.md`, `# Note ${d}/${f}\n\nbody [[note-${(f + 1) % 100}]]\n`);
    git(v.root, "add", "-A");
    git(v.root, "commit", "-q", "-m", "big");
    git(v.root, "checkout", "-q", "main");
    assert.equal(v.listDisk().length, 1);
    const app = await env.launch(v);
    try {
      await app.openFromTree("Open.md");
      await sleep(800);
      let off = env.log().length;
      const t0 = Date.now();
      git(v.root, "checkout", "-q", "big");
      const took = Date.now() - t0;
      await sleep(6000);
      const m1 = await missing(app, v);
      const ov1 = overflowsSince(off);
      off = env.log().length;
      git(v.root, "checkout", "-q", "main");
      await sleep(6000);
      const m2 = await missing(app, v);
      const ov2 = overflowsSince(off);
      console.log(
        JSON.stringify({
          checkoutMs: took,
          toBig: { overflows: ov1, disk: m1.disk, idx: m1.idx, missing: m1.missing.length, extra: m1.extra.length },
          backToMain: { overflows: ov2, disk: m2.disk, idx: m2.idx, missing: m2.missing.length, extra: m2.extra.length },
        }),
      );
      assert.deepEqual({ a: m1.missing.length, b: m2.extra.length }, { a: 0, b: 0 }, `index out of sync after git checkout (overflows ${ov1}/${ov2})`);
    } finally {
      await app.close();
    }
  },
);

test(
  "realistic: copying a 10,000-note folder into the vault",
  async () => {
    const v = env.vault("cp", { "Open.md": "open\n" });
    const src = path.join(env.tmp, "import-src");
    for (let d = 0; d < 100; d++) {
      fs.mkdirSync(path.join(src, `sub-${d}`), { recursive: true });
      for (let f = 0; f < 100; f++) fs.writeFileSync(path.join(src, `sub-${d}`, `n-${f}.md`), `note ${d} ${f}\n`);
    }
    const app = await env.launch(v);
    try {
      await app.openFromTree("Open.md");
      await sleep(800);
      const off = env.log().length;
      const t0 = Date.now();
      execFileSync("cp", ["-r", src, v.p("Imported")]);
      const took = Date.now() - t0;
      await sleep(8000);
      const m = await missing(app, v);
      const ov = overflowsSince(off);
      console.log(JSON.stringify({ cpMs: took, overflows: ov, disk: m.disk, idx: m.idx, missing: m.missing.length }));
      assert.deepEqual(m.missing.length, 0, `${m.missing.length} of ${m.disk} copied paths not indexed (${ov} Q_OVERFLOW)`);
    } finally {
      await app.close();
    }
  },
);
