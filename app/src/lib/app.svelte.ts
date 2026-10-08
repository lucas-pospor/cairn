// Application state: open vault, file list, tabs, panels and dialogs.

import { EditorState } from "@codemirror/state";
import { isolateHistory } from "@codemirror/commands";
import type { EditorView } from "@codemirror/view";
import { tick } from "svelte";
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { backend, vaultUrl } from "./backend";
import { LinkIndex } from "./links";
import { FRONTMATTER_RE, headingsOf } from "./markdown";
import { buildTree } from "./tree";
import { noteExtensions, livePreviewExtension, diskChange, refreshLinks, linkAtCursor, type EditorHooks } from "./editor/setup";
import { refreshEmbeds } from "./editor/livePreview";
import { displayName, isImage, isInside, isMarkdown, isSameOrInside, join, parent, rebase, fileName, resolveRelative } from "./paths";
import { openTarget } from "./opening";
import { errorMessage, isCoreError, type Change, type FileStat, type LinkKind, type SyncStatus, type VaultInfo } from "./types";
import { FONT_NAMES, settings } from "./settings.svelte";
import { commands, displayCombo, isMac as macPlatform } from "./commands";
import { toggleWrap, toggleTask, insertWikilink } from "./editor/format";
import { lineBreaksOf, textWithLineBreaks } from "./editor/lineBreaks";
import { isMobile, isWindows, narrowQuery } from "./platform";
import { changes as heldChanges, wanted as heldWanted, wellFormed, TEXT_LIMIT, type Sent } from "./held";
import type { SessionEndNews } from "./backend";
import { PluginHost } from "./plugins";
import { coreCommands, type CoreHost } from "./corePlugins/core";
import { CORE_PLUGINS } from "./corePlugins";

export type ViewMode = "live" | "source" | "preview" | "split";

const AUTOSAVE_MS = 600;
/** Windows: the text of a note goes to the backend this long after typing stops (see held.ts)... */
const HOLD_MS = 200;
/** ...and at least this often while the user keeps typing. */
const HOLD_MAX_MS = 1000;
/** Toasts on screen at once; a new one pushes out the oldest (a plugin can toast in a loop). */
const MAX_TOASTS = 5;
let nextTabId = 1;

export type TabKind = "note" | "graph" | "image";

export class Tab {
  readonly id = nextTabId++;
  readonly kind: TabKind;
  path = $state("");
  dirty = $state(false);
  mode = $state<ViewMode>("live");
  /** Editing mode to return to when leaving preview. */
  editMode: "live" | "source" = "live";
  /** Set when saving would overwrite a change made outside Cairn. */
  conflict = $state<null | "changed" | "deleted">(null);
  loading = $state(true);
  error = $state<string | null>(null);
  // Not reactive on purpose: CodeMirror objects must not be proxied.
  editorState: EditorState | null = null;
  baseHash: string | null = null;
  /** The file's text at `baseHash`: what a change made on disk is merged against. */
  baseText: string | null = null;
  saving = false;
  saveAgain = false;
  /** The last save failed for another reason than a conflict (a read-only or locked note, a full disk). */
  saveFailed = false;
  saveTimer: ReturnType<typeof setTimeout> | undefined;
  scrollTop = 0;
  /** Line to reveal once the note is shown. */
  revealLine: number | null = null;
  /** Image tabs: shown at its own size rather than fitted to the tab. */
  actualSize = $state(false);
  /** Grows with every change to the text (see held.ts); set when the note loads. */
  edit = 0;
  /** The user discarded this tab's edits, and it is about to go. */
  discarded = false;
  /** Saves since the last one that worked that went again over a file the backend wrote (see movedByTheBackend). */
  movedSaves = 0;

  constructor(path: string, kind: TabKind = "note") {
    this.path = path;
    this.kind = kind;
    this.mode = settings.value.defaultMode;
    if (this.mode !== "preview") this.editMode = this.mode;
    if (kind !== "note") this.loading = false;
  }

  get title() {
    return this.kind === "graph" ? "Graph" : displayName(this.path);
  }
}

export interface Toast {
  id: number;
  message: string;
  kind: "info" | "error";
  /** How long it stays up, in ms; 0 until it is closed. */
  ms: number;
}

export type SettingsSection = "appearance" | "editor" | "files" | "sync" | "core-plugins" | "plugins" | "hotkeys";

export type Dialog =
  | { kind: "prompt"; title: string; value: string; okLabel: string; selectStem?: boolean; resolve: (v: string | null) => void }
  | { kind: "confirm"; title: string; message: string; okLabel: string; danger?: boolean; resolve: (v: boolean) => void }
  | { kind: "choose"; title: string; options: { value: string; label: string }[]; resolve: (v: string | null) => void };

const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

class App {
  vault = $state.raw<VaultInfo | null>(null);
  entries = $state.raw<FileStat[]>([]);
  tree = $derived(buildTree(this.entries));
  linkIndex = $derived(new LinkIndex(this.entries));
  /** All file paths (not folders), for autocomplete. */
  filePaths = $derived(this.entries.filter((e) => e.kind === "file").map((e) => e.path));
  tabs = $state<Tab[]>([]);
  activeId = $state<number | null>(null);
  active = $derived(this.tabs.find((t) => t.id === this.activeId) ?? null);
  expanded = new SvelteSet<string>();
  leftPanel = $state<"files" | "search" | "tags">("files");
  leftOpen = $state(true);
  rightOpen = $state(true);
  switcherOpen = $state(false);
  paletteOpen = $state(false);
  settingsOpen = $state(false);
  /** Section Settings opens on; set by openOverlay(). */
  settingsSection: SettingsSection = "appearance";
  leftTabs = ["files", "search", "tags"] as const;
  searchQuery = $state("");
  toasts = $state<Toast[]>([]);
  dialog = $state<Dialog | null>(null);
  /** Bumped on every vault change; panels re-query when it changes. */
  changeSeq = $state(0);
  /** Bumped on every edit of the active document (for the preview). */
  docSeq = $state(0);
  /** Bumped when the editor's selection changes and there is one (for the word count). */
  selSeq = $state(0);
  /** Bumped to ask the image view for the keyboard focus. */
  imageFocus = $state(0);
  /** For each image file changed since the vault opened, the number of that change (see imageSrc). */
  private imageChanges = new SvelteMap<string, number>();
  private imageChangeCount = 0;
  /**
   * Counts vault openings and folder moves: the same path in another vault
   * is another file, and so is a path a folder was moved onto (the core
   * reports only the folder).
   */
  private imageEpoch = $state(0);
  recent = $state<string[]>([]);
  private syncState = $state<SyncStatus | null>(null);
  /**
   * Sync setup started in Settings: in progress, or how it failed. Kept here
   * so that closing Settings does not lose it.
   */
  syncSetup = $state<{ id: number; busy: boolean; error: string | null; server: string; vaultId: string; device: string } | null>(null);
  /** Path whose version history is shown, if any. Set it with openHistory(). */
  historyFor = $state<string | null>(null);
  /** Vault being opened, if any (a large shared folder on Android takes many seconds). */
  opening = $state<string | null>(null);
  /** The element focused when an overlay was last asked for; see takeOpener(). */
  private opener: Element | null = null;
  readonly isMac = isMac;
  readonly isMobile = isMobile;
  /** Small screen: sidebars are drawers over the editor. */
  narrow = $state(narrowQuery()?.matches ?? false);

  readonly plugins = new PluginHost({
    notePaths: () => this.filePaths.filter((p) => isMarkdown(p)),
    activePath: () => (this.active?.kind === "note" ? this.active.path : null),
    // Only the note on screen: the editor keeps the last note while an image or the graph is shown.
    getSelection: () => {
      const v = this.view;
      if (!v || !this.viewTab || this.viewTab !== this.active) return null;
      const r = v.state.selection.main;
      return v.state.sliceDoc(r.from, r.to);
    },
    replaceSelection: (text) => {
      const v = this.view;
      if (!v || !this.viewTab || this.viewTab !== this.active || this.viewTab.mode === "preview") return false;
      v.dispatch(v.state.replaceSelection(text));
      return true;
    },
    toast: (m, kind) => this.toast(m, kind),
    notice: (m) => this.toast(m, "info", 0),
    disable: (file) => settings.update({ plugins: settings.value.plugins.filter((f) => f !== file) }),
  });

  /** What core plugins can see and do (corePlugins/core.ts). */
  readonly coreHost: CoreHost = {
    files: () => this.filePaths,
    folders: () => this.entries.filter((e) => e.kind === "dir").map((e) => e.path),
    activeNote: () => (this.active?.kind === "note" ? this.active.path : null),
    canInsert: () => this.active?.kind === "note" && this.active.mode !== "preview",
    insert: (path, text) => {
      const v = this.view;
      const tab = this.viewTab;
      if (!v || !tab || tab !== this.active || tab.path !== path || tab.mode === "preview") return false;
      // Its own undo step, also right after typing.
      v.dispatch({ ...v.state.replaceSelection(text), scrollIntoView: true, userEvent: "input", annotations: isolateHistory.of("full") });
      v.focus();
      return true;
    },
    readNote: async (path) => (await backend.readNote(path)).content,
    choose: (title, options) => this.choose({ title, options }),
    createNote: async (path, content) => {
      await backend.createNote(path, content);
      await this.refreshEntries();
    },
    openNote: (path, newTab = false) => this.openNote(path, { newTab }),
    uniquePath: (dir, base) => backend.uniquePath(dir, base, "md"),
    toast: (m, kind) => this.toast(m, kind),
    now: () => new Date(),
    random: () => Math.random(),
  };

  view: EditorView | null = null;
  /** The tab whose document the editor view currently shows. */
  viewTab: Tab | null = null;
  private extensions = noteExtensions(this.hooks());
  readonly livePreview = livePreviewExtension(this.hooks());
  private unlisten: (() => void) | null = null;
  private unlistenClose: (() => void) | null = null;
  /** A window close is being handled (saving, or asking). */
  private closing = false;
  /** The user let the window close (everything saved, or Discard): unloading the page must not ask again. */
  private closeConfirmed = false;
  private refreshTimer: ReturnType<typeof setTimeout> | undefined;
  /** A batch waiting for refreshTimer added, deleted or renamed something. */
  private refreshStructural = false;
  private toastId = 0;
  private syncSetupId = 0;
  private toastTimers = new Map<number, ReturnType<typeof setTimeout>>();
  // Windows: the backend's copy of what is not saved yet (see held.ts). Edit
  // and request numbers start from the clock, so that they keep growing
  // across page loads.
  private editSeq = Date.now() * 1000;
  private heldSeq = this.editSeq;
  /** What the backend took last; null: everything is to be sent, starting with a reset. */
  private heldSent: Map<string, Sent> | null = null;
  /** Counts resets: a request sent before the last one says nothing of what the backend holds now. */
  private heldGen = 0;
  private heldSettings: boolean | null = null;
  /** The requests to the backend, one at a time; a run that has not started yet sends the latest state. */
  private heldQueue: Promise<void> = Promise.resolve();
  private heldQueued = false;
  private heldTimer: ReturnType<typeof setTimeout> | undefined;
  /** When the text waiting to be sent changed first. */
  private heldTextSince = 0;
  /** The settings' failed write was discarded with the window or the notebook. */
  private settingsDiscarded = false;
  /** The notice of a refused sign-out or shutdown, until nothing it names is unsaved. */
  private refusal: { paths: Set<string>; settings: boolean; close: () => void } | null = null;

  /** Sync status of the open vault. */
  get sync() {
    return this.syncState;
  }

  /** A status for another vault is dropped: a sync of the vault the user left can still finish. */
  set sync(st: SyncStatus | null) {
    if (st && st.root !== this.vault?.root) return;
    this.syncState = st;
  }

  private hooks(): EditorHooks {
    return {
      linkIndex: () => this.linkIndex,
      files: () => this.filePaths,
      openLink: (target, subpath, newTab, kind) => this.openLink(target, subpath, newTab, kind),
      openUrl: (url) => this.openUrl(url),
      openImage: (path) => this.openEmbeddedImage(path),
      docChanged: () => this.onEdit(),
      selectionChanged: () => void this.selSeq++,
      notePath: () => this.viewTab?.path ?? "",
      // The active tab, not viewTab: the view takes the new state before viewTab follows.
      noteTitle: () => (this.active?.kind === "note" ? this.active.title : ""),
      saveFiles: (files) => this.saveAttachments(files),
    };
  }

  // ---------- vault ----------

  async init() {
    // Say when a settings change could not be written (a full disk, or a
    // .cairn folder that leads out of the vault), not only in the console.
    settings.onSaveError = (e) => this.toast(`Could not save settings: ${errorMessage(e)}`, "error");
    settings.onFontError = (name, message) =>
      this.toast(`Could not use the font file ${name}: ${message} Notes use the Text font, ${FONT_NAMES[settings.value.fontFamily] ?? "Sans serif"}, instead.`, "error");
    // Lines change height with the font, and the editor keeps the heights it measured.
    settings.onFontChange = () => this.view?.requestMeasure();
    settings.onWriteResult = () => {
      this.settingsDiscarded = false;
      this.heldSync();
    };
    this.unlistenClose = await backend.onCloseRequested(() => this.beforeClose());
    this.registerCommands();
    const q = narrowQuery();
    if (q) {
      // The sidebars as they were in the wide layout: they come back when the
      // window is wide again (drawers opened in between do not count).
      let wide = { left: this.leftOpen, right: this.rightOpen };
      const apply = () => {
        if (q.matches) {
          if (!this.narrow) wide = { left: this.leftOpen, right: this.rightOpen };
          this.leftOpen = this.rightOpen = false;
        } else if (this.narrow) {
          this.leftOpen = wide.left;
          this.rightOpen = wide.right;
        }
        this.narrow = q.matches;
      };
      apply();
      q.addEventListener("change", apply);
    }
    // Pick up changes made while the app was in the background. Desktop has
    // a file watcher too; Android vaults rely on this and on the timer below.
    // Going hidden (to the background, or the page reloading) saves the edits
    // and settings now, and on a phone the session too: Android may kill the
    // app there at any time.
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible" && this.vault) void backend.rescan().catch(() => {});
      else if (this.vault) {
        if (isMobile) this.saveSession();
        void this.flushAll();
        void settings.flush().catch(() => {});
      }
    });
    if (isMobile) {
      setInterval(() => {
        if (document.visibilityState === "visible" && this.vault) void backend.rescan().catch(() => {});
      }, 20000);
    }
    this.unlisten = await backend.onVaultChanged((c) => this.handleChanges(c));
    await backend.onSyncChanged((root, c) => {
      if (root === this.vault?.root) this.handleChanges(c);
    });
    await backend.onSyncStatus((st) => (this.sync = st));
    if (isWindows) {
      await backend.onSessionEnd((round) => this.answerSessionEnd(round));
      await backend.onSessionEndNews((news) => this.sessionEndNews(news));
      this.heldReset();
    }
    this.recent = await backend.recentVaults();
    const start = await backend.startupVault();
    if (start) await this.openVault(start);
  }

  /**
   * Open the vault at `path`. `create`: make the folder first; "ask": ask
   * before making a folder that does not exist (a typed path may be a typo).
   */
  async openVault(path: string, create: boolean | "ask" = false) {
    // A second tap while a slow folder loads must not load it again.
    if (this.opening) return;
    this.opening = path;
    try {
      if (!(await this.flushOrConfirm([...this.tabs], true))) return;
      const info = create === true ? await backend.createVault(path) : await backend.openVault(path);
      this.tabs = [];
      this.activeId = null;
      this.vault = info;
      this.sync = null;
      this.syncSetup = null;
      this.entries = info.entries;
      this.expanded.clear();
      this.imageChanges.clear();
      this.imageEpoch++;
      await settings.load();
      this.settingsDiscarded = false;
      this.heldReset();
      commands.setOverrides(settings.value.hotkeys);
      this.sync = await backend.syncStatus().catch(() => null);
      this.plugins.stopAll();
      // Plugins the vault lists but this device has not approved stay off, without a dialog.
      void this.plugins.sync(settings.value.plugins).then((off) => {
        if (off.length === 1) this.toast("1 plugin in this notebook is off until you turn it on in Settings > Plugins.");
        else if (off.length) this.toast(`${off.length} plugins in this notebook are off until you turn them on in Settings > Plugins.`);
      });
      this.restoreSession();
      this.recent = await backend.recentVaults();
    } catch (e) {
      this.undiscard(this.tabs, true);
      if (create === "ask" && isCoreError(e) && e.kind === "notFound") {
        const ok = await this.confirm({
          title: "Create a new notebook?",
          message: `There is no folder at ${path}. Create it and open it as a new, empty notebook?`,
          okLabel: "Create notebook",
        });
        if (ok) {
          // This open is over; let the one that creates the folder start.
          this.opening = null;
          await this.openVault(path, true);
        }
        return;
      }
      this.toast(`Could not open notebook: ${errorMessage(e)}`, "error");
      this.recent = await backend.recentVaults();
    } finally {
      this.opening = null;
    }
  }

  async closeVault() {
    if (!(await this.flushOrConfirm([...this.tabs], true))) return;
    this.saveSession();
    this.plugins.stopAll();
    settings.reset();
    this.vault = null;
    this.tabs = [];
    this.activeId = null;
    this.entries = [];
    this.settingsDiscarded = false;
    this.heldReset();
    this.recent = await backend.recentVaults();
  }

  /**
   * Set up sync with the details from Settings (see `syncSetup`). A vault the
   * server does not have is only created when the user says so: a mistyped
   * name would start a second, empty vault.
   */
  async setupSync(args: { server: string; token: string; vaultId: string; device: string; passphrase: string }) {
    const id = ++this.syncSetupId;
    this.syncSetup = { id, busy: true, error: null, server: args.server, vaultId: args.vaultId, device: args.device };
    try {
      if (!(await backend.syncVaultExists(args))) {
        const create =
          this.syncSetup?.id === id &&
          (await this.confirm({
            title: "New notebook",
            message: `There's no notebook called ${args.vaultId.trim()} on this server. Create it?`,
            okLabel: "Create",
          }));
        if (this.syncSetup?.id !== id) return;
        if (!create) {
          // Back to the form, to correct the name.
          this.syncSetup = { ...this.syncSetup, busy: false };
          return;
        }
      }
      if (this.syncSetup?.id !== id) return;
      this.sync = await backend.syncSetup(args);
      if (this.syncSetup?.id === id) this.syncSetup = null;
    } catch (e) {
      if (this.syncSetup?.id === id) this.syncSetup = { ...this.syncSetup, busy: false, error: errorMessage(e) };
    }
  }

  cancelSyncSetup() {
    this.syncSetup = null;
    void backend.syncCancel().catch(() => {});
  }

  async forgetVault(path: string) {
    await backend.forgetVault(path);
    this.recent = await backend.recentVaults();
  }

  private sessionKey() {
    return this.vault ? `cairn.session:${this.vault.root}` : null;
  }

  saveSession() {
    const key = this.sessionKey();
    if (!key) return;
    // Image tabs are not kept: Cairn 1.0.0 and 1.1.0 would open them as notes.
    const kept = this.tabs.filter((t) => t.kind !== "image");
    const data = {
      tabs: kept.map((t) => ({ path: t.path, mode: t.mode, kind: t.kind })),
      active: this.active && kept.includes(this.active) ? this.active.path : null,
      expanded: [...this.expanded],
    };
    try {
      localStorage.setItem(key, JSON.stringify(data));
    } catch {
      /* storage unavailable */
    }
  }

  private restoreSession() {
    const key = this.sessionKey();
    if (!key) return;
    try {
      const raw = localStorage.getItem(key);
      if (!raw) return;
      const data = JSON.parse(raw) as {
        tabs: { path: string; mode: ViewMode; kind?: TabKind }[];
        active: string | null;
        expanded: string[];
      };
      const exists = new Set(this.entries.map((e) => e.path));
      for (const d of data.expanded ?? []) if (exists.has(d)) this.expanded.add(d);
      for (const t of data.tabs ?? []) {
        if (t.kind === "graph") {
          this.tabs.push(new Tab("", "graph"));
          continue;
        }
        // A kind this version does not know, and images (a note tab whose
        // file was renamed to one), are left out.
        if ((t.kind && t.kind !== "note") || !exists.has(t.path) || isImage(t.path)) continue;
        const tab = new Tab(t.path);
        tab.mode = t.mode ?? settings.value.defaultMode;
        if (tab.mode === "live" || tab.mode === "source") tab.editMode = tab.mode;
        this.tabs.push(tab);
        void this.loadTab(tab);
      }
      const active = this.tabs.find((t) => t.path === data.active) ?? this.tabs[0];
      this.activeId = active?.id ?? null;
    } catch {
      /* ignore broken session data */
    }
  }

  // ---------- changes from disk or from our own commands ----------

  private handleChanges(changes: Change[]) {
    for (const c of changes) {
      if (c.type === "renamed") {
        if (c.entry.kind === "file") {
          this.fileIsBack(c.entry.path);
          // A file moved over an open image.
          this.imageChanged(c.entry.path);
        } else this.imageEpoch++;
        // The notice of a refused sign-out follows the notes it names.
        const r = this.refusal;
        if (r) r.paths = new Set([...r.paths].map((p) => (isSameOrInside(p, c.from) ? rebase(p, c.from, c.entry.path) : p)));
        for (const t of [...this.tabs]) {
          if (!isSameOrInside(t.path, c.from)) continue;
          t.path = rebase(t.path, c.from, c.entry.path);
          // Renamed to a name that is not an image's (it is never dirty).
          if (t.kind === "image" && !isImage(t.path)) this.closeTab(t, { skipSave: true });
        }
        for (const d of [...this.expanded]) {
          if (isSameOrInside(d, c.from)) {
            this.expanded.delete(d);
            this.expanded.add(rebase(d, c.from, c.entry.path));
          }
        }
      } else if (c.type === "deleted") {
        for (const t of [...this.tabs]) {
          if (!isSameOrInside(t.path, c.path)) continue;
          if (t.dirty) t.conflict = "deleted";
          else this.closeTab(t, { skipSave: true });
        }
      } else if (c.type === "created") {
        if (c.entry.kind === "file") {
          this.fileIsBack(c.entry.path);
          this.imageChanged(c.entry.path);
        } else this.imageEpoch++;
      } else if (c.type === "modified") {
        const t = this.tabs.find((x) => x.kind === "note" && x.path === c.entry.path);
        if (t && !t.dirty && !t.saving && !t.loading) void this.reloadIfChanged(t);
        this.imageChanged(c.entry.path);
      }
    }
    // Content edits do not change the file list; only structural changes
    // need a refetch (which rebuilds the tree and link index). An image's
    // new size is shown in its tab, so a changed image counts too. The flag
    // outlives the timer it was set for, so a content-only batch that
    // follows within 60 ms does not drop the refetch.
    if (changes.some((c) => c.type !== "modified")) this.heldSync();
    if (changes.some((c) => c.type !== "modified" || isImage(c.entry.path))) this.refreshStructural = true;
    clearTimeout(this.refreshTimer);
    this.refreshTimer = setTimeout(() => {
      if (this.refreshStructural) {
        this.refreshStructural = false;
        void this.refreshEntries();
      } else {
        this.view?.dispatch({ effects: refreshEmbeds.of(null) });
        this.changeSeq++;
      }
    }, 60);
  }

  /**
   * A file is at `path` again: tabs whose file was deleted would now replace
   * it. (One that is saving is recreating it itself, or will get a conflict.)
   */
  private fileIsBack(path: string) {
    for (const t of this.tabs) if (t.conflict === "deleted" && t.path === path && !t.saving) t.conflict = "changed";
  }

  /** The file at `path` changed on disk: an image tab showing it, now or later, loads it again (see imageSrc). */
  private imageChanged(path: string) {
    if (isImage(path)) this.imageChanges.set(path, ++this.imageChangeCount);
  }

  /**
   * The URL an image tab loads `path` from. It changes with each change to
   * the file: the web view keeps images it has shown in memory by URL, and
   * would show the old picture again.
   */
  imageSrc(path: string): string {
    return `${vaultUrl(path)}?v=${this.imageEpoch}.${this.imageChanges.get(path) ?? 0}`;
  }

  private async refreshEntries() {
    if (!this.vault) return;
    try {
      this.entries = await backend.listEntries();
    } catch (e) {
      console.warn(e);
    }
    this.view?.dispatch({ effects: [refreshLinks.of(null), refreshEmbeds.of(null)] });
    this.changeSeq++;
    this.saveSession();
  }

  private async reloadIfChanged(tab: Tab) {
    try {
      const n = await backend.readNote(tab.path);
      if (tab.dirty) return; // user typed meanwhile; saving will sort it out
      tab.baseHash = n.hash;
      tab.baseText = n.content;
      const current = this.fileTextOf(tab);
      if (current === n.content) return;
      this.replaceDoc(tab, n.content);
    } catch (e) {
      console.warn("reload failed", e);
    }
  }

  /**
   * Set a tab's document to text from disk; the cursor stays at the same text.
   * Only `undoable` replacements (the user's own choice) can be undone.
   */
  private replaceDoc(tab: Tab, content: string, undoable = false) {
    const isActive = this.viewTab === tab && this.view;
    const st = isActive ? this.view!.state : tab.editorState;
    if (!st) return;
    const spec = diskChange(st, content, undoable);
    if (isActive) this.view!.dispatch(spec);
    else tab.editorState = st.update(spec).state;
    tab.edit = ++this.editSeq;
    if (this.active === tab) this.docSeq++;
    this.heldSync();
  }

  // ---------- tabs ----------

  private stateOf(tab: Tab): EditorState | null {
    return this.viewTab === tab && this.view ? this.view.state : tab.editorState;
  }

  docOf(tab: Tab): string {
    return this.stateOf(tab)?.doc.toString() ?? "";
  }

  /** The text selected in a tab the editor shows (several ranges on lines of their own), or null when nothing is. */
  selectionOf(tab: Tab): string | null {
    if (this.viewTab !== tab || !this.view || tab.mode === "preview") return null;
    const st = this.view.state;
    const ranges = st.selection.ranges.filter((r) => !r.empty);
    return ranges.length ? ranges.map((r) => st.sliceDoc(r.from, r.to)).join("\n") : null;
  }

  /** A tab's text as it is saved: with the note's own line breaks. */
  private fileTextOf(tab: Tab): string {
    const st = this.stateOf(tab);
    return st ? textWithLineBreaks(st) : "";
  }

  private async loadTab(tab: Tab) {
    tab.loading = true;
    tab.error = null;
    try {
      const n = await backend.readNote(tab.path);
      tab.baseHash = n.hash;
      tab.baseText = n.content;
      // Start below the frontmatter so Live Preview shows it as properties.
      const fm = FRONTMATTER_RE.exec(n.content);
      // (A CRLF line break is one character in the editor.)
      const anchor = fm ? fm[0].replace(/\r\n/g, "\n").length : 0;
      tab.editorState = EditorState.create({
        doc: n.content,
        selection: { anchor },
        extensions: [this.extensions, lineBreaksOf(n.content)],
      });
      tab.edit = ++this.editSeq;
    } catch (e) {
      tab.error = errorMessage(e);
    } finally {
      tab.loading = false;
    }
  }

  /**
   * Open a file of the vault: a note or an image in a tab (it replaces the
   * active tab of the same kind unless `newTab`), anything else in the
   * system's default app (see openTarget).
   */
  async openNote(path: string, opts: { newTab?: boolean; line?: number; heading?: string | null } = {}) {
    const target = openTarget(path, isMobile);
    if (target === "image") {
      this.openImage(path, opts.newTab);
      return;
    }
    if (target !== "note") {
      await this.openAttachment(path);
      return;
    }
    let tab = this.tabs.find((t) => t.kind === "note" && t.path === path);
    if (!tab) {
      const current = this.active;
      // Reuse the active tab, unless it still holds edits that could not be
      // saved (conflict or failed write): those stay in their own tab.
      if (current && current.kind === "note" && !opts.newTab && (await this.flush(current))) {
        this.stashActive();
        if (this.viewTab === current) this.viewTab = null;
        current.path = path;
        current.conflict = null;
        current.dirty = false;
        current.saveFailed = false;
        current.editorState = null;
        tab = current;
      } else {
        tab = new Tab(path);
        this.tabs.push(tab);
      }
      await this.loadTab(tab);
    }
    if (opts.heading && tab.editorState) {
      const line = findHeadingLine(tab.editorState.doc.toString(), opts.heading);
      if (line != null) opts.line = line;
    }
    tab.revealLine = opts.line ?? null;
    this.activate(tab);
    // activate() saves only when the active tab changes, not when this tab
    // now shows another note.
    this.saveSession();
    // Small screen: close the drawer the note was chosen from.
    const covered = this.narrow && (this.leftOpen || this.rightOpen);
    if (this.narrow) this.leftOpen = this.rightOpen = false;
    // A tab not in view yet: the editor pane shows it, then reveals the line or focuses the editor.
    if (this.viewTab !== tab) return;
    // The editor cannot take focus while a drawer covers it (inert): wait until it is gone.
    if (covered) await tick();
    if (tab.revealLine != null) this.reveal(tab);
    else if (covered && tab.mode !== "preview") this.view?.focus();
  }

  /** Copy the view's state back into the tab it shows, before switching. */
  stashActive() {
    const cur = this.viewTab;
    if (cur && this.view) {
      cur.editorState = this.view.state;
      cur.scrollTop = this.view.scrollDOM.scrollTop;
    }
  }

  /** Called by the editor pane when it shows a tab's document. */
  showInView(tab: Tab) {
    if (!this.view || !tab.editorState || this.viewTab === tab) return;
    this.stashActive();
    this.view.setState(tab.editorState);
    this.viewTab = tab;
  }

  activate(tab: Tab) {
    if (this.activeId === tab.id) return;
    this.activeId = tab.id;
    this.saveSession();
  }

  reveal(tab: Tab) {
    const view = this.view;
    if (!view || tab.revealLine == null || this.viewTab !== tab) return;
    const doc = view.state.doc;
    const ln = Math.min(Math.max(tab.revealLine + 1, 1), doc.lines);
    const line = doc.line(ln);
    tab.revealLine = null;
    view.dispatch({ selection: { anchor: line.from }, scrollIntoView: true });
    view.focus();
  }

  async closeTab(tab: Tab, opts: { skipSave?: boolean } = {}) {
    if (!opts.skipSave && !(await this.flushOrConfirm([tab]))) return;
    const i = this.tabs.indexOf(tab);
    if (i < 0) return;
    if (this.viewTab === tab) {
      this.stashActive();
      this.viewTab = null;
    }
    clearTimeout(tab.saveTimer);
    this.tabs.splice(i, 1);
    if (this.activeId === tab.id) {
      const next = this.tabs[i] ?? this.tabs[i - 1] ?? null;
      this.activeId = next?.id ?? null;
    }
    this.saveSession();
    this.heldSync();
  }

  cycleTab(dir: 1 | -1) {
    if (this.tabs.length < 2 || !this.active) return;
    const i = this.tabs.indexOf(this.active);
    this.activate(this.tabs[(i + dir + this.tabs.length) % this.tabs.length]);
  }

  setMode(mode: ViewMode) {
    const t = this.active;
    if (!t) return;
    if (mode === "live" || mode === "source") t.editMode = mode;
    t.mode = mode;
    this.saveSession();
  }

  /** Ctrl+E: switch between editing and reading. */
  toggleMode() {
    const t = this.active;
    if (!t) return;
    this.setMode(t.mode === "preview" ? t.editMode : "preview");
  }

  /** Switch the editor between Live Preview and plain source. */
  toggleSource() {
    const t = this.active;
    if (!t) return;
    this.setMode(t.mode === "source" ? "live" : "source");
  }

  // ---------- editing & saving ----------

  private onEdit() {
    const tab = this.viewTab;
    if (!tab) return;
    const first = !tab.dirty;
    tab.dirty = true;
    tab.discarded = false;
    tab.edit = ++this.editSeq;
    this.docSeq++;
    clearTimeout(tab.saveTimer);
    tab.saveTimer = setTimeout(() => void this.save(tab), AUTOSAVE_MS);
    // The first edit of a saved note at once: the backend then knows it holds edits.
    this.heldSync(first ? "now" : "text");
  }

  async save(tab: Tab, force = false) {
    clearTimeout(tab.saveTimer);
    if (!tab.dirty || (tab.conflict && !force)) return;
    if (tab.saving) {
      tab.saveAgain = true;
      return;
    }
    tab.saving = true;
    const content = this.fileTextOf(tab);
    const edit = tab.edit;
    const sentBase = tab.baseHash;
    // "Save my version" of a deleted note: never over a file that is back.
    const recreate = force && tab.conflict === "deleted";
    try {
      const r = recreate
        ? await backend.recreateNote(tab.path, content, edit)
        : await backend.writeNote(tab.path, content, force ? null : tab.baseHash, false, edit);
      tab.baseHash = r.hash;
      tab.baseText = content;
      tab.conflict = null;
      tab.saveFailed = false;
      tab.movedSaves = 0;
      if (this.fileTextOf(tab) === content) tab.dirty = false;
    } catch (e) {
      if (isCoreError(e) && e.kind === "conflict") {
        if (!force && (await this.movedByTheBackend(tab, sentBase))) tab.saveAgain = true;
        else if (!force && (await this.mergeFromDisk(tab))) tab.saveAgain = true;
        else if (recreate) tab.conflict = "changed";
        else {
          // Ask the disk whether the note is still there: the file list can lag
          // behind (on Android it refreshes only with the periodic rescan).
          const gone = await backend.readNote(tab.path).then(
            () => false,
            (err) => isCoreError(err) && err.kind === "notFound",
          );
          tab.conflict = gone ? "deleted" : "changed";
          // Drop the note from the file tree now rather than at the next rescan.
          if (gone && this.entries.some((x) => x.path === tab.path)) void backend.rescan().catch(() => {});
        }
      } else {
        tab.saveFailed = true;
        this.toast(`Could not save ${tab.title}: ${errorMessage(e)}`, "error");
      }
    } finally {
      tab.saving = false;
      this.heldSync();
      if (tab.saveAgain) {
        tab.saveAgain = false;
        void this.save(tab);
      }
    }
  }

  /**
   * A save found the file changed: true when that was Cairn's own doing, so
   * the save can simply go again over the new file. Either the backend
   * reported writing the note meanwhile (the tab took the new base already),
   * or, on Windows, it wrote the note at the end of a session that went on.
   */
  private async movedByTheBackend(tab: Tab, sentBase: string | null): Promise<boolean> {
    // A few times at most per save that works: then the merge or the banner decides.
    if (++tab.movedSaves > 3) return false;
    if (tab.baseHash !== sentBase) return true;
    if (!isWindows || sentBase === null) return false;
    const to = await backend.sessionMoved(tab.path, sentBase).catch(() => null);
    if (tab.baseHash !== sentBase) return true;
    if (!to || to === sentBase) return false;
    tab.baseHash = to;
    return true;
  }

  /**
   * The file changed on disk since the tab loaded it (sync pulled an edit
   * from another device, or another app wrote it) while the tab had unsaved
   * edits. Merge that change into the editor text, as sync merges notes, so
   * that saving again keeps both. False when the changes overlap or are
   * too large to merge quickly (or the file cannot be read): then the user
   * chooses in the conflict banner.
   */
  private async mergeFromDisk(tab: Tab): Promise<boolean> {
    const base = tab.baseText;
    if (base === null) return false;
    try {
      const disk = await backend.readNote(tab.path);
      // Nothing new to merge (the save failed for another reason): saving
      // again would only fail again.
      if (disk.hash === tab.baseHash) return false;
      for (;;) {
        const mine = this.fileTextOf(tab);
        const merged = await backend.mergeText(base, mine, disk.content);
        // Typed on meanwhile: merge the new text instead.
        if (this.fileTextOf(tab) !== mine) continue;
        if (merged === null) return false;
        tab.baseHash = disk.hash;
        tab.baseText = disk.content;
        this.replaceDoc(tab, merged);
        return true;
      }
    } catch (e) {
      console.warn("merge failed", e);
      return false;
    }
  }

  /** Save now if needed. Resolves to true when the tab has no unsaved edits left. */
  async flush(tab: Tab): Promise<boolean> {
    if (tab.dirty && !tab.conflict) await this.save(tab);
    while (tab.saving) await new Promise((r) => setTimeout(r, 10));
    return !tab.dirty;
  }

  async flushAll() {
    for (const t of this.tabs) await this.flush(t);
  }

  /**
   * Save tabs that are about to go away (and the settings, when the vault or
   * the window is). If some still hold edits that could not be saved (a
   * conflict, or a failed write), ask before throwing them away. Resolves to
   * false when the user keeps them.
   */
  async flushOrConfirm(tabs: Tab[], withSettings = false): Promise<boolean> {
    const unsaved: Tab[] = [];
    for (const t of tabs) if (!(await this.flush(t))) unsaved.push(t);
    const names = unsaved.map((t) => `"${t.title}"`);
    if (withSettings) {
      try {
        await settings.flush();
      } catch (e) {
        this.toast(`Could not save settings: ${errorMessage(e)}`, "error");
        names.push("the settings");
      }
    }
    if (!names.length) return true;
    const ok = await this.confirm({
      title: "Unsaved changes",
      message: `Discard unsaved changes to ${names.join(", ")}?`,
      okLabel: "Discard",
      danger: true,
    });
    if (!ok && unsaved.length && this.tabs.includes(unsaved[0])) this.activate(unsaved[0]);
    if (ok) {
      // Discarded: the backend lets them go before they close or are deleted.
      for (const t of unsaved) t.discarded = true;
      if (names.includes("the settings")) this.settingsDiscarded = true;
      await this.heldSend();
    }
    return ok;
  }

  /**
   * The window is about to close: save notes and settings first, and ask
   * before dropping edits that could not be saved. Resolves to false to keep
   * the window open.
   */
  async beforeClose(): Promise<boolean> {
    // A second close request while the first is still saving or asking.
    if (this.closing) return false;
    this.closing = true;
    try {
      if (!(await this.flushOrConfirm([...this.tabs], true))) return false;
      this.saveSession();
      this.closeConfirmed = true;
      await this.heldSend();
      return true;
    } finally {
      this.closing = false;
    }
  }

  /**
   * Tabs whose edits a page reload would lose: a conflict, or a failed save.
   * Edits still waiting for autosave do not count, as the page going hidden
   * saves them (see init).
   */
  unsavableTabs(): Tab[] {
    return this.tabs.filter((t) => t.dirty && (t.conflict !== null || t.saveFailed));
  }

  /**
   * The page is about to unload: keep the session, and while a tab holds
   * edits that could not be saved, or a change of the settings could not be
   * written, have the web view ask before a reload drops them. Closing the
   * window asked already (beforeClose).
   */
  beforeUnload(e: { preventDefault(): void; returnValue: unknown }) {
    this.saveSession();
    if (this.closeConfirmed || (!this.unsavableTabs().length && !settings.saveFailed)) return;
    e.preventDefault();
    // Older engines ask only when returnValue is set.
    e.returnValue = true;
  }

  /** Conflict resolution: keep the editor text and overwrite the file. */
  async keepMine(tab: Tab) {
    tab.dirty = true;
    tab.discarded = false;
    await this.save(tab, true);
  }

  /** Conflict resolution: discard edits and load the file from disk. */
  async loadTheirs(tab: Tab) {
    try {
      const n = await backend.readNote(tab.path);
      tab.baseHash = n.hash;
      tab.baseText = n.content;
      // Ctrl+Z afterwards brings the discarded edits back.
      this.replaceDoc(tab, n.content, true);
      tab.dirty = false;
      tab.conflict = null;
      tab.saveFailed = false;
      this.heldSync();
    } catch (e) {
      this.toast(errorMessage(e), "error");
    }
  }

  // ---------- the end of a Windows session ----------

  /** What a Discard was for did not happen: the backend holds these edits again. */
  private undiscard(tabs: Tab[], settingsToo: boolean) {
    for (const t of tabs) t.discarded = false;
    if (settingsToo) this.settingsDiscarded = false;
    this.heldSync();
  }

  /** What the backend should hold, from the tabs (see held.ts). */
  private heldWanted() {
    if (this.closeConfirmed) return new Map();
    return heldWanted(
      this.tabs.map((t) => ({
        kind: t.kind,
        path: t.path,
        dirty: t.dirty,
        conflict: t.conflict,
        saveFailed: t.saveFailed,
        baseHash: t.baseHash,
        edit: t.edit,
        loaded: !t.loading && this.stateOf(t) !== null,
        discarded: t.discarded,
      })),
    );
  }

  /** A note's text for the backend, or null: too large (unless `limit` is Infinity) or not well formed. */
  private heldText(path: string, limit = TEXT_LIMIT): string | null {
    const tab = this.tabs.find((t) => t.kind === "note" && t.path === path && t.dirty && !t.discarded);
    const st = tab ? this.stateOf(tab) : null;
    if (!st || st.doc.length + st.doc.lines > limit) return null;
    const text = textWithLineBreaks(st);
    return wellFormed(text) ? text : null;
  }

  private heldSettingsFailed() {
    return settings.saveFailed && !this.settingsDiscarded && !this.closeConfirmed;
  }

  /** Start over with the backend: the page loaded, or another notebook opened or closed. */
  private heldReset() {
    if (!isWindows) return;
    this.heldGen++;
    this.heldSent = null;
    this.heldSync();
  }

  /**
   * Tell the backend what changed: "now", or "text" for typing, which waits
   * until HOLD_MS after the last key, HOLD_MAX_MS at most.
   */
  heldSync(when: "now" | "text" = "now") {
    if (!isWindows) return;
    if (when === "text") {
      const now = Date.now();
      if (!this.heldTextSince) this.heldTextSince = now;
      const due = Math.min(now + HOLD_MS, this.heldTextSince + HOLD_MAX_MS);
      clearTimeout(this.heldTimer);
      this.heldTimer = setTimeout(() => void this.heldSend(), due - now);
      return;
    }
    queueMicrotask(() => void this.heldSend());
  }

  /** Send what changed now, one request at a time; resolves once the backend took it. */
  private heldSend(): Promise<void> {
    if (!isWindows) return Promise.resolve();
    clearTimeout(this.heldTimer);
    this.heldTextSince = 0;
    if (!this.heldQueued) {
      this.heldQueued = true;
      this.heldQueue = this.heldQueue.then(() => {
        this.heldQueued = false;
        return this.heldSendOnce();
      });
    }
    return this.heldQueue;
  }

  private async heldSendOnce() {
    const gen = this.heldGen;
    const reset = this.heldSent === null;
    const { notes, next } = heldChanges(this.heldSent ?? new Map(), this.heldWanted(), (p) => this.heldText(p));
    const settingsFailed = this.heldSettingsFailed();
    if (!reset && !notes.length && settingsFailed === this.heldSettings) {
      this.checkRefusal();
      return;
    }
    try {
      const reply = await backend.sessionHold({ seq: ++this.heldSeq, root: this.vault?.root ?? null, reset, settingsFailed, notes });
      if (reset) this.editSeq = this.heldSeq = Math.max(this.editSeq, this.heldSeq, reply.floor);
      if (reply.applied && gen === this.heldGen) {
        this.heldSent = next;
        this.heldSettings = settingsFailed;
      }
    } catch (e) {
      // Everything again, a little later.
      console.warn("session end: the backend did not take what the page holds", e);
      this.heldSent = null;
      this.heldSettings = null;
      setTimeout(() => this.heldSync(), 1000);
    }
    this.checkRefusal();
  }

  /**
   * Windows is ending the session and the backend asks for what the page
   * holds: every note with unsaved edits, with its text whatever its size,
   * at once and without waiting for anything.
   */
  answerSessionEnd(round: number) {
    const want = this.heldWanted();
    const { notes, next } = heldChanges(new Map(), want, (p) => this.heldText(p, Infinity));
    const settingsFailed = this.heldSettingsFailed();
    const gen = this.heldGen;
    backend.sessionHold({ seq: ++this.heldSeq, root: this.vault?.root ?? null, round, settingsFailed, notes }).then(
      (reply) => {
        if (reply.applied && gen === this.heldGen) {
          this.heldSent = next;
          this.heldSettings = settingsFailed;
        }
      },
      (e) => console.warn("session end: no answer", e),
    );
  }

  /**
   * The session went on after Windows asked to end it. Notes the backend
   * wrote take the new file as their base and are saved again (the editor
   * may hold newer text). After a no, a notice says what is not saved.
   */
  sessionEndNews(news: SessionEndNews) {
    for (const w of news.written) {
      const tabs = this.tabs.filter((t) => t.kind === "note" && t.path === w.path);
      const t = tabs[0];
      if (tabs.length !== 1 || !t.dirty || t.baseHash !== w.from) continue;
      t.baseHash = w.to;
      if (!t.conflict) void this.save(t);
    }
    if (news.refused.length) {
      this.refusal?.close();
      const names = news.refused.map((r) => r.name).join(", ");
      const close = this.toast(
        `Windows was about to sign out or shut down, but changes to ${names} are not saved. Save or discard them, then try again.`,
        "error",
        0,
      );
      const paths = new Set(news.refused.flatMap((r) => (r.path === null ? [] : [r.path])));
      this.refusal = { paths, settings: news.refused.some((r) => r.path === null), close };
      const first = this.tabs.find((t) => t.kind === "note" && paths.has(t.path) && t.dirty);
      if (first) this.activate(first);
    }
    this.heldSync();
  }

  /** The notice of a refusal goes once nothing it names is unsaved. */
  private checkRefusal() {
    const r = this.refusal;
    if (!r) return;
    const left =
      this.tabs.some((t) => t.kind === "note" && t.dirty && !t.discarded && r.paths.has(t.path)) || (r.settings && this.heldSettingsFailed());
    if (left) return;
    r.close();
    this.refusal = null;
  }

  // ---------- links ----------

  /** Follow a link in the active note; `kind` is "markdown" for [text](path) links. */
  async openLink(target: string, subpath: string | null, newTab = false, kind: LinkKind = "wiki") {
    const source = this.active?.path ?? "";
    if (!target) {
      if (subpath && this.active) await this.openNote(this.active.path, { heading: subpath });
      return;
    }
    const resolved = await backend.resolveLink(target, source, kind);
    if (resolved) {
      await this.openNote(resolved, { newTab, heading: subpath });
      return;
    }
    // Broken link: create the note, like Obsidian does. ./ and ../ are
    // relative to the linking note's folder.
    let path = target.replace(/\\/g, "/").replace(/^\/+/, "").normalize("NFC");
    if (path.split("/").some((s) => s === "." || s === "..")) path = resolveRelative(parent(source), path) ?? path;
    if (!/\.(md|markdown)$/i.test(path)) path += ".md";
    try {
      await backend.createNote(path, "");
      await this.openNote(path, { newTab });
    } catch (e) {
      this.toast(`Could not create ${path}: ${errorMessage(e)}`, "error");
    }
  }

  async openUrl(url: string) {
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } catch (e) {
      this.toast(`Could not open ${url}: ${errorMessage(e)}`, "error");
    }
  }

  // ---------- attachments ----------

  /**
   * Show an image in an image tab: the one already showing it, else the
   * active tab if it is an image tab (unless `newTab`), else a new tab.
   */
  openImage(path: string, newTab = false) {
    let tab = this.tabs.find((t) => t.kind === "image" && t.path === path);
    if (!tab) {
      const current = this.active;
      if (current?.kind === "image" && !newTab) {
        current.path = path;
        current.actualSize = false;
        tab = current;
      } else {
        tab = new Tab(path, "image");
        this.tabs.push(tab);
      }
    }
    this.activate(tab);
    // Small screen: close the drawer the image was chosen from.
    if (this.narrow) this.leftOpen = this.rightOpen = false;
    this.imageFocus++;
  }

  /**
   * Open an image embedded in a note: clicked in the reading view, or
   * Ctrl/Cmd+clicked or middle-clicked in Live Preview. Raw HTML in a note
   * can set the path too, so only an image of this vault opens.
   */
  openEmbeddedImage(path: string, newTab = false) {
    if (!isImage(path) || !this.entries.some((e) => e.kind === "file" && e.path === path)) return;
    this.openImage(path, newTab);
  }

  /** Open a non-note file with the system's default app. */
  async openAttachment(path: string) {
    if (isMobile) {
      this.toast(`${fileName(path)} is an attachment. On Android, Cairn cannot open attachments in other apps yet.`);
      return;
    }
    try {
      await backend.openExternally(path);
    } catch (e) {
      this.toast(`Could not open ${fileName(path)}: ${errorMessage(e)}`, "error");
    }
  }

  /** Show a file or folder of the vault in the system's file manager. */
  async revealInFileManager(path: string) {
    try {
      await backend.revealInFileManager(path);
    } catch (e) {
      this.toast(`Could not show ${fileName(path)} in the file manager: ${errorMessage(e)}`, "error");
    }
  }

  /**
   * Store files (pasted or dropped) in the attachment folder and return the
   * Markdown to insert for them.
   */
  async saveAttachments(files: File[]): Promise<string> {
    const folder = settings.value.attachmentFolder.trim().replace(/^\/+|\/+$/g, "");
    const parts: string[] = [];
    for (const f of files) {
      let name = f.name;
      if (!name || name === "image.png" || name === "blob") {
        const ext = (f.type.split("/")[1] ?? "png").replace("jpeg", "jpg").replace("svg+xml", "svg");
        const d = new Date();
        const stamp = `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, "0")}${String(d.getDate()).padStart(2, "0")}${String(d.getHours()).padStart(2, "0")}${String(d.getMinutes()).padStart(2, "0")}${String(d.getSeconds()).padStart(2, "0")}`;
        name = `Pasted image ${stamp}.${ext}`;
      }
      name = name.replace(/[\\/:*?"<>|[\]#^]/g, "-");
      try {
        const bytes = new Uint8Array(await f.arrayBuffer());
        const path = await backend.saveAttachment(folder, name, bytes);
        const sameName = this.entries.some((e) => e.kind === "file" && fileName(e.path) === fileName(path) && e.path !== path);
        const text = sameName ? path : fileName(path);
        parts.push(isMarkdown(path) ? `[[${text.replace(/\.md$/i, "")}]]` : `![[${text}]]`);
      } catch (e) {
        this.toast(`Could not save ${name}: ${errorMessage(e)}`, "error");
      }
    }
    return parts.join("\n");
  }

  // ---------- graph ----------

  openGraph() {
    let tab = this.tabs.find((t) => t.kind === "graph");
    if (!tab) {
      tab = new Tab("", "graph");
      this.tabs.push(tab);
    }
    this.activate(tab);
  }

  // ---------- commands ----------

  private editorCmd(fn: (v: EditorView) => boolean) {
    return () => {
      const v = this.view;
      if (!v || !this.active || this.active.kind !== "note" || this.active.mode === "preview") return;
      v.focus();
      fn(v);
    };
  }

  /** Open the link at the cursor: the keyboard's Ctrl+click. */
  private followLink(v: EditorView): boolean {
    const link = linkAtCursor(v.state);
    if (!link) {
      this.toast("No link at the cursor.");
      return false;
    }
    if (link.kind === "url") void this.openUrl(link.url);
    else void this.openLink(link.target, link.subpath, false, link.kind);
    return true;
  }

  /**
   * Open the quick switcher, the command palette or Settings (null: none).
   * Opening one closes the others, so there is never an overlay left behind
   * one that has focus.
   */
  openOverlay(which: "switcher" | "palette" | "settings" | null, section: SettingsSection = "appearance") {
    this.opener = document.activeElement;
    this.settingsSection = section;
    this.switcherOpen = which === "switcher";
    this.paletteOpen = which === "palette";
    this.settingsOpen = which === "settings";
  }

  /**
   * A command's first key as hints show it ("Ctrl+N"), or "" when it has
   * none. Read through the settings, so hints follow changed hotkeys.
   */
  hotkeyHint(id: string): string {
    const k = commands.keysFor(id, settings.value.hotkeys)[0];
    return k ? displayCombo(k) : "";
  }

  /** Show the version history of `path`, closing the switcher, palette and Settings. */
  openHistory(path: string) {
    this.openOverlay(null);
    this.historyFor = path;
  }

  /**
   * For an overlay's script: the element to give focus back to when it
   * closes, i.e. the one focused when it was asked for. Taken then and not
   * when the overlay is created: by that time an overlay it replaces has been
   * removed, and the focus with it.
   */
  takeOpener(): Element | null {
    const el = this.opener ?? document.activeElement;
    this.opener = null;
    return el;
  }

  private registerCommands() {
    const editor = () => !!this.active && this.active.kind === "note" && this.active.mode !== "preview";
    commands.register([
      { id: "app:quick-switcher", name: "Open quick switcher", run: () => this.openOverlay("switcher"), defaultKeys: ["Mod+O"] },
      { id: "app:command-palette", name: "Open command palette", run: () => this.openOverlay("palette"), defaultKeys: ["Mod+P"] },
      { id: "app:settings", name: "Open settings", run: () => this.openOverlay("settings"), defaultKeys: ["Mod+,"] },
      { id: "app:graph", name: "Open graph view", run: () => this.openGraph(), defaultKeys: ["Mod+G"] },
      {
        id: "app:search",
        name: "Search in all notes",
        run: () => {
          this.leftOpen = true;
          this.leftPanel = "search";
          queueMicrotask(() => document.querySelector<HTMLInputElement>("[data-testid=search-input]")?.focus());
        },
        defaultKeys: ["Mod+Shift+F"],
      },
      { id: "app:tags", name: "Show tags", run: () => void ((this.leftOpen = true), (this.leftPanel = "tags")) },
      { id: "app:files", name: "Show files", run: () => void ((this.leftOpen = true), (this.leftPanel = "files")) },
      { id: "app:toggle-left", name: "Toggle left sidebar", run: () => void (this.leftOpen = !this.leftOpen) },
      { id: "app:toggle-right", name: "Toggle right sidebar", run: () => void (this.rightOpen = !this.rightOpen) },
      { id: "app:close-vault", name: "Switch notebook", run: () => this.closeVault() },
      {
        id: "sync:now",
        name: "Sync now",
        run: async () => {
          if (!this.sync?.configured) {
            this.openOverlay("settings", "sync");
            return;
          }
          this.sync = await backend.syncNow();
          // The user asked for this sync: say why it failed, which the status bar does not.
          if (this.sync.state === "error" && this.sync.lastError) this.toast(`Sync failed: ${this.sync.lastError}`, "error");
        },
      },
      {
        id: "sync:history",
        name: "Show version history of current note",
        run: () => {
          if (this.active?.kind !== "note") return;
          this.openHistory(this.active.path);
        },
        available: () => this.active?.kind === "note" && !!this.sync?.configured,
      },
      { id: "note:new", name: "Create new note", run: () => this.newNote(), defaultKeys: ["Mod+N"] },
      { id: "note:new-folder", name: "Create new folder", run: () => this.newFolder(this.contextDir()) },
      {
        id: "note:save",
        name: "Save current note",
        run: () => void (this.active && this.save(this.active)),
        defaultKeys: ["Mod+S"],
        available: editor,
      },
      {
        id: "note:rename",
        name: "Rename current note",
        run: () => {
          const t = this.active;
          if (!t || t.kind !== "note") return;
          this.leftOpen = true;
          this.leftPanel = "files";
          for (let d = parent(t.path); d; d = parent(d)) this.expanded.add(d);
          this.renaming = t.path;
        },
        available: () => this.active?.kind === "note",
      },
      {
        id: "note:move",
        name: "Move current note to…",
        run: () => void (this.active && this.moveToFolderPrompt(this.active.path)),
        available: () => this.active?.kind === "note",
      },
      {
        id: "note:delete",
        name: "Delete current note",
        run: () => void (this.active && this.remove(this.active.path)),
        available: () => this.active?.kind === "note",
      },
      {
        id: "note:reveal",
        name: "Reveal current note in file manager",
        run: () => void (this.active && this.revealInFileManager(this.active.path)),
        available: () => this.active?.kind === "note" && !isMobile,
      },
      {
        id: "tab:close",
        name: "Close tab",
        run: () => void (this.active && this.closeTab(this.active)),
        defaultKeys: ["Mod+W"],
      },
      { id: "tab:next", name: "Next tab", run: () => this.cycleTab(1), defaultKeys: [macPlatform ? "Ctrl+Tab" : "Mod+Tab", "Mod+PageDown"] },
      {
        id: "tab:prev",
        name: "Previous tab",
        run: () => this.cycleTab(-1),
        defaultKeys: [macPlatform ? "Ctrl+Shift+Tab" : "Mod+Shift+Tab", "Mod+PageUp"],
      },
      {
        id: "view:toggle-reading",
        name: "Toggle reading view",
        run: () => this.toggleMode(),
        defaultKeys: ["Mod+E"],
        available: () => this.active?.kind === "note",
      },
      {
        id: "view:toggle-source",
        name: "Toggle Live Preview / source mode",
        run: () => this.toggleSource(),
        available: () => this.active?.kind === "note",
      },
      { id: "view:split", name: "Show source and preview side by side", run: () => this.setMode("split"), available: () => this.active?.kind === "note" },
      { id: "editor:bold", name: "Toggle bold", run: this.editorCmd((v) => toggleWrap(v, "**")), defaultKeys: ["Mod+B"], available: editor },
      { id: "editor:italic", name: "Toggle italic", run: this.editorCmd((v) => toggleWrap(v, "*")), defaultKeys: ["Mod+I"], available: editor },
      { id: "editor:code", name: "Toggle inline code", run: this.editorCmd((v) => toggleWrap(v, "`")), defaultKeys: ["Mod+Shift+C"], available: editor },
      { id: "editor:strike", name: "Toggle strikethrough", run: this.editorCmd((v) => toggleWrap(v, "~~")), available: editor },
      { id: "editor:task", name: "Toggle checkbox", run: this.editorCmd(toggleTask), defaultKeys: ["Mod+L"], available: editor },
      { id: "editor:link", name: "Insert internal link", run: this.editorCmd(insertWikilink), defaultKeys: ["Mod+K"], available: editor },
      { id: "editor:follow-link", name: "Follow link under cursor", run: this.editorCmd((v) => this.followLink(v)), defaultKeys: ["Alt+Enter"], available: editor },
      ...coreCommands(CORE_PLUGINS, this.coreHost),
    ]);
  }

  // ---------- file operations ----------

  /** Folder where "new note" should go, based on the selection. */
  contextDir(): string {
    return this.active ? parent(this.active.path) : "";
  }

  async newNote(dir = this.contextDir(), opts: { newTab?: boolean } = { newTab: true }) {
    if (this.narrow) {
      // The tree is in a closed drawer on small screens: ask for the name.
      const name = await this.prompt({ title: "New note", value: "Untitled", okLabel: "Create" });
      if (!name) return;
      try {
        const path = await backend.uniquePath(dir, name.trim().replace(/\.md$/i, ""), "md");
        await backend.createNote(path, "");
        await this.refreshEntries();
        await this.openNote(path, { newTab: opts.newTab });
        queueMicrotask(() => this.view?.focus());
      } catch (e) {
        this.toast(errorMessage(e), "error");
      }
      return;
    }
    try {
      const path = await backend.uniquePath(dir, "Untitled", "md");
      await backend.createNote(path, "");
      if (dir) this.expanded.add(dir);
      await this.refreshEntries();
      // Set before opening so the editor does not steal focus from the rename box.
      this.renaming = path;
      await this.openNote(path, { newTab: opts.newTab });
    } catch (e) {
      this.toast(errorMessage(e), "error");
    }
  }

  async createNoteNamed(name: string, opts: { newTab?: boolean } = {}) {
    let path = name.trim().replace(/\\/g, "/").replace(/^\/+/, "");
    if (!path) return;
    if (!/\.md$/i.test(path)) path += ".md";
    try {
      await backend.createNote(path, "");
      await this.refreshEntries();
      await this.openNote(path, opts);
    } catch (e) {
      this.toast(errorMessage(e), "error");
    }
  }

  /** Move without dragging (touch, keyboard): pick the destination folder from a list. */
  async moveToFolderPrompt(path: string) {
    const dirs = ["", ...this.entries.filter((e) => e.kind === "dir" && !isSameOrInside(e.path, path)).map((e) => e.path).sort()];
    const choice = await this.choose({
      title: `Move "${displayName(path)}" to`,
      options: dirs.map((d) => ({ value: d, label: d || "Notebook folder" })),
    });
    if (choice == null) return;
    await this.moveInto(path, choice);
  }

  async newFolder(dir = "") {
    const name = await this.prompt({ title: "New folder", value: "", okLabel: "Create" });
    if (!name) return;
    try {
      const path = join(dir, name.trim());
      await backend.createFolder(path);
      if (dir) this.expanded.add(dir);
      this.expanded.add(path);
    } catch (e) {
      this.toast(errorMessage(e), "error");
    }
  }

  /** Path currently being renamed inline in the file tree. */
  renaming = $state<string | null>(null);

  async rename(from: string, newName: string) {
    // Called from both Enter and blur; only the first call counts.
    if (this.renaming !== from) return;
    this.renaming = null;
    // Back to the editor or the image; on a small screen the drawer stays open over it and the tree keeps focus.
    if (!this.narrow) {
      if (this.active?.kind === "image") this.imageFocus++;
      else queueMicrotask(() => this.view?.focus());
    }
    const entry = this.entries.find((e) => e.path === from);
    if (!entry) return;
    let name = newName.trim();
    if (!name) return;
    // A slash would turn the rename into a move to another folder.
    if (/[\\/]/.test(name)) {
      this.toast(errorMessage({ kind: "invalidName", detail: name }), "error");
      return;
    }
    if (entry.kind === "file" && isMarkdown(from) && !/\.(md|markdown)$/i.test(name)) name += ".md";
    const to = join(parent(from), name);
    if (to === from) return;
    await this.move(from, to);
  }

  async move(from: string, to: string) {
    if (from === to) return;
    if (isInside(to, from)) {
      this.toast("Cannot move a folder into itself.", "error");
      return;
    }
    try {
      for (const t of this.tabs) if (isSameOrInside(t.path, from)) await this.flush(t);
      await backend.renameEntry(from, to);
    } catch (e) {
      this.toast(errorMessage(e), "error");
    }
  }

  async moveInto(from: string, dir: string) {
    if (parent(from) === dir) return;
    await this.move(from, join(dir, fileName(from)));
    if (dir) this.expanded.add(dir);
  }

  async remove(path: string) {
    const entry = this.entries.find((e) => e.path === path);
    if (!entry) return;
    const what = entry.kind === "dir" ? `the folder "${fileName(path)}" and everything in it` : `"${displayName(path)}"`;
    const ok = await this.confirm({
      title: "Delete",
      message: `Move ${what} to the trash?`,
      okLabel: "Delete",
      danger: true,
    });
    if (!ok) return;
    // Save open tabs first so the trash gets their latest text, and close
    // them only once the delete has worked.
    const affected = this.tabs.filter((t) => isSameOrInside(t.path, path));
    if (!(await this.flushOrConfirm(affected))) return;
    try {
      await backend.deleteEntry(path);
    } catch (e) {
      this.undiscard(affected, false);
      this.toast(errorMessage(e), "error");
      return;
    }
    for (const t of affected) await this.closeTab(t, { skipSave: true });
  }

  // ---------- dialogs & toasts ----------

  prompt(opts: { title: string; value: string; okLabel: string; selectStem?: boolean }): Promise<string | null> {
    return new Promise((resolve) => {
      this.dialog = { kind: "prompt", ...opts, resolve };
    });
  }

  confirm(opts: { title: string; message: string; okLabel: string; danger?: boolean }): Promise<boolean> {
    return new Promise((resolve) => {
      this.dialog = { kind: "confirm", ...opts, resolve };
    });
  }

  choose(opts: { title: string; options: { value: string; label: string }[] }): Promise<string | null> {
    return new Promise((resolve) => {
      this.dialog = { kind: "choose", ...opts, resolve };
    });
  }

  /**
   * Show a toast for `ms` (0: until the returned function is called). The
   * same timed message again while it is up, such as a save that keeps
   * failing, shows no second toast: the one that is up starts its time
   * again, unless it is held. Toasts that stay until they are closed are
   * never merged, as each has its own caller that closes it.
   */
  toast(message: string, kind: Toast["kind"] = "info", ms = kind === "error" ? 7000 : 3500) {
    const same = ms > 0 ? this.toasts.find((t) => t.ms > 0 && t.kind === kind && t.message === message) : undefined;
    if (same) {
      if (this.toastTimers.has(same.id)) this.releaseToast(same.id);
      return () => this.dismissToast(same.id);
    }
    const id = ++this.toastId;
    this.toasts.push({ id, message, kind, ms });
    if (this.toasts.length > MAX_TOASTS) {
      // The oldest timed toasts go first: one that stays until it is closed,
      // such as the notice of a refused sign-out, only when no timed one is left.
      const older = this.toasts.slice(0, -1);
      const out = [...older.filter((t) => t.ms), ...older.filter((t) => !t.ms)].slice(0, this.toasts.length - MAX_TOASTS);
      for (const old of out) this.holdToast(old.id);
      const gone = new Set(out.map((t) => t.id));
      this.toasts = this.toasts.filter((t) => !gone.has(t.id));
    }
    this.releaseToast(id);
    return () => this.dismissToast(id);
  }

  /** Keep a toast up while the pointer or the keyboard focus is on it. */
  holdToast(id: number) {
    clearTimeout(this.toastTimers.get(id));
    this.toastTimers.delete(id);
  }

  /** Start (again) the time a toast stays up: its own `ms`; 0 keeps it until it is closed. */
  releaseToast(id: number) {
    const t = this.toasts.find((x) => x.id === id);
    if (!t) return;
    clearTimeout(this.toastTimers.get(id));
    if (t.ms) this.toastTimers.set(id, setTimeout(() => this.dismissToast(id), t.ms));
  }

  dismissToast(id: number) {
    this.holdToast(id);
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  dispose() {
    this.unlisten?.();
    this.unlistenClose?.();
  }
}

export function findHeadingLine(doc: string, heading: string): number | null {
  const want = heading.trim().toLowerCase();
  return headingsOf(doc).find((h) => h.text.toLowerCase() === want)?.line ?? null;
}

/** The search for a tag. A frontmatter tag can contain spaces: quoted. */
export function tagQuery(tag: string): string {
  return /\s/.test(tag) ? `tag:"${tag}"` : `tag:${tag}`;
}

export const app = new App();
