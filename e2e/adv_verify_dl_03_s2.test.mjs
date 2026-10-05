// Reproduction for FINDING-009:
// "Closing the window loses whatever was typed in the last 600 ms."
//
// The other reproduction (adv_verify_dl_03.test.mjs) closes the window with
// WebDriver "Close Window". These
// tests instead press the real GTK title-bar close button (the client-side
// "Close" GtkButton) through AT-SPI. That is the path a user takes when they
// click X or when the compositor asks the window to close (Alt+F4):
// GTK delete-event -> tao CloseRequested -> the UI saves (or asks), then
// destroys the window -> last window gone -> process exit. No in-app Quit
// command exists.
//
// Cases:
//  - control: X pressed 1.5 s after the last key -> edit is on disk, process exits;
//  - X pressed right after the last key -> the edit is on disk too;
//  - ~8 s of steady typing (keys 180 ms apart, so the 600 ms debounce never
//    fires) then X -> the whole burst is on disk (the defect lost all of it,
//    not only "the last 600 ms").
//
// Needs a private session bus with an AT-SPI bus and registry, and GTK must not
// have NO_AT_BRIDGE set:
//   dbus-run-session -- bash -c '/usr/lib/at-spi-bus-launcher --launch-immediately & sleep 1;
//     /usr/lib/at-spi2-registryd & sleep 1;
//     env -u NO_AT_BRIDGE scripts/e2e-headless.sh e2e/adv_verify_dl_03_s2.test.mjs'
// If the close button cannot be found over AT-SPI the tests skip.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { Env, sleep, APP } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("vdl03s2");
});
after(async () => {
  await env?.dispose();
});

function appPids() {
  const want = `XDG_CONFIG_HOME=${path.join(env.tmp, "config")}`;
  const out = [];
  for (const d of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(d)) continue;
    try {
      if (path.resolve(fs.readlinkSync(`/proc/${d}/exe`)) !== path.resolve(APP)) continue;
      if (fs.readFileSync(`/proc/${d}/environ`, "utf8").split("\0").includes(want)) out.push(Number(d));
    } catch {}
  }
  return out;
}

function alive(pid) {
  try {
    return !/\) [ZX] /.test(fs.readFileSync(`/proc/${pid}/stat`, "utf8"));
  } catch {
    return false;
  }
}

// Finds the app's title-bar "Close" button over AT-SPI, prints READY, presses it
// when a line arrives on stdin, prints DONE.
const PY = String.raw`
import sys, time, gi
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi
pid = int(sys.argv[1])
def find():
    d = Atspi.get_desktop(0)
    for i in range(d.get_child_count()):
        a = d.get_child_at_index(i)
        if a is None: continue
        try:
            if a.get_process_id() != pid: continue
        except Exception: continue
        q = [a]
        while q:
            n = q.pop(0)
            try:
                role = n.get_role_name(); name = n.get_name() or ''
            except Exception: continue
            if role in ('push button', 'button') and name == 'Close':
                return n
            if role in ('document web', 'document frame'):
                continue
            try: k = n.get_child_count()
            except Exception: k = 0
            for j in range(k):
                try: c = n.get_child_at_index(j)
                except Exception: c = None
                if c is not None: q.append(c)
    return None
btn = None
end = time.time() + 15
while time.time() < end and btn is None:
    btn = find()
    if btn is None: time.sleep(0.3)
if btn is None:
    print('NOBUTTON', flush=True); sys.exit(0)
act = btn.get_action_iface()
names = [act.get_action_name(i) for i in range(act.get_n_actions())]
idx = names.index('click') if 'click' in names else 0
print('READY', flush=True)
sys.stdin.readline()
act.do_action(idx)
print('DONE', flush=True)
`;

class Closer {
  constructor(pid) {
    this.p = spawn("python3", ["-c", PY, String(pid)], { stdio: ["pipe", "pipe", "pipe"] });
    this.out = "";
    this.err = "";
    this.p.stdout.on("data", (d) => (this.out += d));
    this.p.stderr.on("data", (d) => (this.err += d));
  }
  async ready() {
    const end = Date.now() + 20000;
    while (Date.now() < end) {
      if (this.out.includes("READY")) return true;
      if (this.out.includes("NOBUTTON") || this.p.exitCode !== null) return false;
      await sleep(50);
    }
    return false;
  }
  async click() {
    this.p.stdin.write("go\n");
    const end = Date.now() + 5000;
    while (Date.now() < end && !this.out.includes("DONE")) await sleep(10);
    return this.out.includes("DONE");
  }
  kill() {
    try {
      this.p.kill();
    } catch {}
  }
}

async function launchWithCloser(files, rel) {
  const v = env.vault("v", files);
  const before = new Set(appPids());
  const app = await env.launch(v);
  const pids = appPids().filter((p) => !before.has(p));
  assert.equal(pids.length, 1, `expected one new app process, got ${JSON.stringify(pids)}`);
  const pid = pids[0];
  await app.openFromTree(rel);
  await app.source();
  await sleep(150);
  const closer = new Closer(pid);
  if (!(await closer.ready())) {
    closer.kill();
    await app.close();
    return { skip: `AT-SPI close button not found: ${closer.out} ${closer.err.slice(0, 400)}` };
  }
  return { v, app, pid, closer };
}

async function waitExit(pid) {
  for (let i = 0; i < 50 && alive(pid); i++) await sleep(100);
  return !alive(pid);
}

test("control: real title-bar X 1.5 s after typing keeps the edit and exits the process", async (t) => {
  const r = await launchWithCloser({ "A.md": "alpha\n" }, "A.md");
  if (r.skip) return t.skip(r.skip);
  const { v, app, pid, closer } = r;
  try {
    await app.typeEnd(" last words");
    await sleep(1500);
    assert.ok(await closer.click(), "close button pressed");
    assert.ok(await waitExit(pid), "process exits after the title-bar close");
    assert.equal(v.read("A.md"), "alpha\n last words");
  } finally {
    closer.kill();
    await app.close();
  }
});

test(
  "real title-bar X right after typing keeps the edit",
  async (t) => {
    const r = await launchWithCloser({ "A.md": "alpha\n" }, "A.md");
    if (r.skip) return t.skip(r.skip);
    const { v, app, pid, closer } = r;
    try {
      await app.typeEnd(" last words");
      const t0 = Date.now();
      assert.ok(await closer.click(), "close button pressed");
      const dt = Date.now() - t0;
      const exited = await waitExit(pid);
      await sleep(500);
      console.log(`[DL-03 s2] click ${dt} ms after last key; exited=${exited}; disk=${JSON.stringify(v.read("A.md"))}`);
      assert.equal(v.read("A.md"), "alpha\n last words");
    } finally {
      closer.kill();
      await app.close();
    }
  },
);

test(
  "~8 s of steady typing (keys 180 ms apart) then the real title-bar X keeps all of it",
  async (t) => {
    const r = await launchWithCloser({ "A.md": "alpha\n" }, "A.md");
    if (r.skip) return t.skip(r.skip);
    const { v, app, pid, closer } = r;
    try {
      await app.focusEnd();
      const text = " a sentence typed steadily over about eight seconds";
      const actions = [];
      for (const ch of text) actions.push({ type: "keyDown", value: ch }, { type: "keyUp", value: ch }, { type: "pause", duration: 180 });
      const t0 = Date.now();
      await app.s.cmd("POST", "/actions", { actions: [{ type: "key", id: "kb", actions }] });
      const typingMs = Date.now() - t0;
      const midDisk = v.read("A.md");
      const editor = await app.editorText();
      assert.ok(await closer.click(), "close button pressed");
      const exited = await waitExit(pid);
      await sleep(500);
      const disk = v.read("A.md");
      console.log(
        `[DL-03 s2] typed ${typingMs} ms; editor=${JSON.stringify(editor)}; disk before close=${JSON.stringify(midDisk)}; after=${JSON.stringify(disk)}; exited=${exited}`,
      );
      assert.equal(disk, "alpha\n" + text);
    } finally {
      closer.kill();
      await app.close();
    }
  },
);
