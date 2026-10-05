// Approve plugins on "this device" for an e2e run, as Settings > Plugins does
// when the user turns one on. A plugin a vault's settings.json lists starts
// only with such an approval (FINDING-005). The app keeps approvals in
// <XDG_CONFIG_HOME>/app.cairn.notes/plugin-approvals.json, by vault root and
// plugin file: the SHA-256 of the file and the permissions it declares
// (app/src-tauri/src/config.rs, app/src/lib/plugins.ts). The app reads that
// file on every use, so this can run while the app is open.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

/** Approve `files` (names in <vault>/.cairn/plugins) for the vault at `vaultDir`, with their current content. */
export function approvePlugins(configHome, vaultDir, files) {
  const store = path.join(configHome, "app.cairn.notes", "plugin-approvals.json");
  let all = {};
  try {
    all = JSON.parse(fs.readFileSync(store, "utf8"));
  } catch {}
  const root = fs.realpathSync(vaultDir);
  all[root] ??= {};
  for (const file of files) {
    const source = fs.readFileSync(path.join(vaultDir, ".cairn/plugins", file), "utf8");
    const declared = /^\s*\/\/\s*@permissions\s+(.+)$/m.exec(source)?.[1] ?? "";
    const permissions = declared.split(/[\s,]+/).filter((p) => ["read", "write", "editor"].includes(p));
    all[root][file] = { hash: crypto.createHash("sha256").update(source, "utf8").digest("hex"), permissions };
  }
  fs.mkdirSync(path.dirname(store), { recursive: true });
  fs.writeFileSync(store, JSON.stringify(all, null, 2));
}
