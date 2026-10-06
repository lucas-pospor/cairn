// Regression test for FINDING-121 ("Open" on the Welcome screen
// silently created a vault for a path that did not exist).
//
// With the defect, the typed-path form called
// app.openVault(path, create = true), and nothing expanded "~". Typing a
// path the way Linux users usually write it, "~/Something/Notes", created a
// literal folder named "~" in the app's working directory and opened it as a
// new empty vault, with no question asked. The test checks that no literal
// "~" folder is created and no vault is opened.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_ax_29.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { AxApp } from "./adv_a11y_lib.mjs";

const app = new AxApp("cairn-ax29v-");

before(async () => {
  await app.start();
});

after(async () => {
  await app.stop();
});

/** Working directory of this test's app process (found by its XDG_CONFIG_HOME). */
function appCwd() {
  let fallback = null;
  for (const pid of fs.readdirSync("/proc").filter((d) => /^\d+$/.test(d))) {
    try {
      if (fs.readFileSync(`/proc/${pid}/comm`, "utf8").trim() !== "cairn") continue;
      const cwd = fs.readlinkSync(`/proc/${pid}/cwd`);
      if (fs.readFileSync(`/proc/${pid}/environ`, "utf8").includes(app.tmp)) return cwd;
      fallback = cwd;
    } catch {}
  }
  return fallback;
}

async function openTyped(p) {
  await app.exec(
    `const i = document.querySelector('[data-testid=vault-path]'); i.value = arguments[0]; i.dispatchEvent(new Event('input', { bubbles: true })); document.querySelector('[data-testid=vault-open]').click(); return 1`,
    p,
  );
}

test("FINDING-121: typing '~/…/Notes' and pressing Open neither creates a literal '~' folder in the app's working directory nor opens an empty vault", async () => {
  const cwd = appCwd();
  assert.ok(cwd, "found the app process");
  const unique = `cairn-ax29-verify-${process.pid}`;
  const tildeDir = path.join(cwd, "~");
  const hadTilde = fs.existsSync(tildeDir);
  const literal = path.join(tildeDir, unique, "Notes");
  try {
    await app.palette("switch notebook");
    await app.s.waitFor(`return !!document.querySelector('[data-testid=vault-path]')`, { message: "welcome screen" });
    await openTyped(`~/${unique}/Notes`);
    await app.s.waitFor(`return !!document.querySelector('[data-testid=file-tree]')`, { timeout: 8000, message: "workspace opened" }).catch(() => {});
    const st = await app.exec(`return { workspace: !!document.querySelector('[data-testid=file-tree]'), dialog: !!document.querySelector('[role=dialog]'), empty: document.querySelector('.tree .empty')?.textContent ?? null, toast: [...document.querySelectorAll('.toast')].map(t => t.textContent.trim()) }`);
    const created = fs.existsSync(literal);
    console.log(JSON.stringify({ appCwd: cwd, typed: `~/${unique}/Notes`, literalFolderCreated: created, literal, ...st }, null, 1));
    assert.ok(!created && !st.workspace, `"~/${unique}/Notes" was created as ${literal} and opened as an empty vault without confirmation`);
  } finally {
    fs.rmSync(path.join(tildeDir, unique), { recursive: true, force: true });
    if (!hadTilde) {
      try {
        fs.rmdirSync(tildeDir); // only succeeds if empty
      } catch {}
    }
  }
});
