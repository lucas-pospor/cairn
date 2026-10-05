// Reproduction for FINDING-045:
// the editor drops CR from every line break when a note is loaded, so the first
// autosave rewrites CRLF, lone-CR and mixed files to LF. Also pins down where
// the conversion happens (editor, not the core) and that merely opening a CRLF
// note does not rewrite it.
//
// Run: scripts/e2e-headless.sh e2e/adv_verify_dl_06.test.mjs

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { Env, sleep } from "./adv_dataloss_helpers.mjs";

let env;
before(async () => {
  env = await Env.create("vdl06");
});
after(async () => {
  await env?.dispose();
});

async function editAndSave(app, rel, insert = "X") {
  await app.openFromTree(rel);
  await app.source();
  await app.insertEnd(insert);
  await app.waitSaved();
  await sleep(200);
}

test("control: the core's write_note keeps CRLF byte for byte; the editor doc has no CR right after load; opening alone does not rewrite", async () => {
  const v = env.vault("v", { "crlf.md": "a\r\nb\r\n", "core.md": "x\n", "Z.md": "z\n" });
  const app = await env.launch(v);
  try {
    const r = await app.invoke("write_note", { path: "core.md", content: "p\r\nq\r\n", baseHash: null });
    assert.ok(r.ok, JSON.stringify(r));
    assert.equal(v.read("core.md"), "p\r\nq\r\n", "core rewrote CRLF");
    await app.openFromTree("crlf.md");
    const text = await app.editorText();
    assert.equal(text, "a\nb\n", "editor document right after load");
    await sleep(1500);
    assert.equal(v.read("crlf.md"), "a\r\nb\r\n", "file rewritten without an edit");
  } finally {
    await app.close();
  }
});

test(
  "lone-CR (classic Mac) note keeps its line endings after an edit",
  async () => {
    const v = env.vault("v", { "cr.md": "one\rtwo\rthree\r", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await editAndSave(app, "cr.md");
      const disk = v.read("cr.md");
      assert.equal(disk, "one\rtwo\rthree\rX", `lone CR rewritten: ${JSON.stringify(disk)}`);
    } finally {
      await app.close();
    }
  },
);

test(
  "mixed-line-ending note keeps untouched lines after an edit",
  async () => {
    const v = env.vault("v", { "mixed.md": "a\r\nb\nc\rd\r\n", "Z.md": "z\n" });
    const app = await env.launch(v);
    try {
      await editAndSave(app, "mixed.md");
      const disk = v.read("mixed.md");
      assert.equal(disk, "a\r\nb\nc\rd\r\nX", `mixed rewritten: ${JSON.stringify(disk)}`);
    } finally {
      await app.close();
    }
  },
);
