// The only module that talks to Tauri. Everything else imports from here.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  Backlinks,
  Change,
  FileStat,
  GraphData,
  NoteContent,
  NoteInfo,
  TagCount,
  SyncStatus,
  HistoryEntry,
  LinkKind,
  OutgoingLink,
  SearchHit,
  VaultInfo,
  WriteResult,
} from "./types";
import type { PluginApproval } from "./plugins";
import { isAndroid } from "./platform";

/** Base64 of `bytes`, in pieces so a large file does not overflow the call stack. */
function base64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/** URL under which the web view can load a vault file (images, media). */
export function vaultUrl(path: string): string {
  const enc = path.split("/").map(encodeURIComponent).join("/");
  const ua = typeof navigator !== "undefined" ? navigator.userAgent : "";
  return /Windows|Android/i.test(ua) ? `http://vault.localhost/${enc}` : `vault://localhost/${enc}`;
}

export const backend = {
  startupVault: () => invoke<string | null>("startup_vault"),
  openVault: (path: string) => invoke<VaultInfo>("open_vault", { path }),
  createVault: (path: string) => invoke<VaultInfo>("create_vault", { path }),
  recentVaults: () => invoke<string[]>("recent_vaults"),
  forgetVault: (path: string) => invoke<void>("forget_vault", { path }),
  listEntries: () => invoke<FileStat[]>("list_entries"),
  // `inVault` (plugins): refuse a path that a symlink leads out of the vault.
  readNote: (path: string, inVault = false) => invoke<NoteContent>("read_note", { path, inVault }),
  writeNote: (path: string, content: string, baseHash: string | null, inVault = false) =>
    invoke<WriteResult>("write_note", { path, content, baseHash, inVault }),
  /** Write a note whose file is gone; a "conflict" error if a file is there again. */
  recreateNote: (path: string, content: string) => invoke<WriteResult>("recreate_note", { path, content }),
  /** Merge unsaved text with a change made on disk since `base`, as sync does; null when they overlap. */
  mergeText: (base: string, ours: string, theirs: string) => invoke<string | null>("merge_text", { base, ours, theirs }),
  createNote: (path: string, content = "", inVault = false) => invoke<WriteResult>("create_note", { path, content, inVault }),
  createFolder: (path: string) => invoke<Change[]>("create_folder", { path }),
  renameEntry: (from: string, to: string) => invoke<Change[]>("rename_entry", { from, to }),
  deleteEntry: (path: string) => invoke<Change[]>("delete_entry", { path }),
  uniquePath: (dir: string, base: string, ext: string) => invoke<string>("unique_path", { dir, base, ext }),
  search: (query: string, limit = 100) => invoke<SearchHit[]>("search", { query, limit }),
  backlinks: (path: string) => invoke<Backlinks[]>("backlinks", { path }),
  outgoingLinks: (path: string) => invoke<OutgoingLink[]>("outgoing_links", { path }),
  // What a click on a link in `source` opens, as backlinks and the graph see it.
  resolveLink: (target: string, source: string, kind: LinkKind = "wiki") =>
    invoke<string | null>("resolve_link", { target, source, kind }),
  rescan: () => invoke<Change[]>("rescan"),
  uiReady: () => invoke<number>("ui_ready"),
  readConfig: (name: string) => invoke<string | null>("read_config", { name }),
  writeConfig: (name: string, content: string) => invoke<void>("write_config", { name, content }),
  listConfig: (dir: string) => invoke<string[]>("list_config", { dir }),
  /** A `.cairn/` file that is not text (a font); rejects with notFound if missing, or if larger than `max` bytes. */
  readConfigBytes: (name: string, max: number) => invoke<ArrayBuffer>("read_config_bytes", { name, max }),
  /** Raw bytes on the desktop; Android passes no raw body to a command, so base64 in JSON there. */
  writeConfigBytes: (name: string, bytes: Uint8Array) =>
    isAndroid
      ? invoke<void>("write_config_bytes", { name, data: base64(bytes) })
      : invoke<void>("write_config_bytes", bytes, { headers: { "x-name": encodeURIComponent(name) } }),
  /** Move a `.cairn/` file to the trash: true if it did, false if there was no such file. */
  trashConfig: (name: string) => invoke<boolean>("trash_config", { name }),
  /** Plugins turned on on this device for the open vault, by file name. */
  pluginApprovals: () => invoke<Record<string, PluginApproval>>("plugin_approvals"),
  setPluginApproval: (file: string, approval: PluginApproval | null) => invoke<void>("set_plugin_approval", { file, approval }),
  /** A non-note file's text; null if it is too big or not text (a PDF, an archive). */
  readTextFile: (path: string) => invoke<string | null>("read_text_file", { path }),
  tags: () => invoke<TagCount[]>("tags"),
  graph: (includeUnresolved: boolean) => invoke<GraphData>("graph", { includeUnresolved }),
  noteInfo: (path: string) => invoke<NoteInfo | null>("note_info", { path }),
  openExternally: (path: string) => invoke<void>("open_externally", { path }),
  /** What openExternally would do with the file, without opening it. */
  openExternallyCheck: (path: string) => invoke<"opens" | "type" | "linktype" | "executable" | "notfile">("open_externally_check", { path }),
  revealInFileManager: (path: string) => invoke<void>("reveal_in_file_manager", { path }),
  /** Store pasted or dropped bytes as a new file in `dir`; returns its vault path. */
  saveAttachment: (dir: string, name: string, bytes: Uint8Array) =>
    // Tauri cannot send raw bytes to a command on Android: base64 there.
    invoke<string>("save_attachment", isAndroid ? { data: base64(bytes) } : bytes, {
      headers: { "x-dir": encodeURIComponent(dir), "x-name": encodeURIComponent(name) },
    }),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  syncSetup: (args: { server: string; token: string; vaultId: string; device: string; passphrase: string }) =>
    invoke<SyncStatus>("sync_setup", { args }),
  /** Whether the server has this vault; it is not created. */
  syncVaultExists: (args: { server: string; token: string; vaultId: string }) =>
    invoke<boolean>("sync_vault_exists", { args: { server: args.server, token: args.token, vaultId: args.vaultId } }),
  syncNow: () => invoke<SyncStatus>("sync_now"),
  syncCancel: () => invoke<void>("sync_cancel"),
  syncDisconnect: () => invoke<SyncStatus>("sync_disconnect"),
  syncHistory: (path: string) => invoke<HistoryEntry[]>("sync_history", { path }),
  syncRevision: (seq: number) => invoke<{ path: string; text: string }>("sync_revision", { seq }),
  syncRestore: (path: string, seq: number) => invoke<void>("sync_restore", { path, seq }),
  defaultDeviceName: () => invoke<string>("default_device_name"),
  platform: () => invoke<string>("platform"),
  setWindowTitle: (title: string) => getCurrentWindow().setTitle(title),
  appVaults: () => invoke<[string, string][]>("app_vaults"),
  pickFolder: () => invoke<{ uri: string | null; name: string | null }>("pick_folder"),
  onSyncStatus: (cb: (s: SyncStatus) => void): Promise<UnlistenFn> => listen<SyncStatus>("sync-status", (e) => cb(e.payload)),
  onVaultChanged: (cb: (changes: Change[]) => void): Promise<UnlistenFn> =>
    listen<Change[]>("vault-changed", (e) => cb(e.payload)),
  /** Changes made by a sync, with the root of the vault it synced. */
  onSyncChanged: (cb: (root: string, changes: Change[]) => void): Promise<UnlistenFn> =>
    listen<{ root: string; changes: Change[] }>("sync-changed", (e) => cb(e.payload.root, e.payload.changes)),
  /**
   * Ask `canClose` before the window closes (title-bar X, Alt+F4, the
   * desktop asking the app to quit). The window stays open when it resolves
   * to false.
   */
  onCloseRequested: (canClose: () => Promise<boolean>): Promise<UnlistenFn> =>
    getCurrentWindow().onCloseRequested(async (e) => {
      if (!(await canClose())) e.preventDefault();
    }),
};
