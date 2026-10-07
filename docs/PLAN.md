# Cairn plan

Cairn is a local-first Markdown notes app. A notebook is an ordinary folder of `.md` files. Cairn reads and writes those files directly, and everything else it keeps (search index, link graph, sync state) is a cache that can be thrown away and rebuilt from the folder.

This document records the decisions, the reasons behind them, and the status of each version. It is updated at the end of every version.

## 1. Stack

| Layer | Choice |
|---|---|
| App shell (desktop + Android) | Tauri 2 |
| Core logic | Rust crate `cairn-core` (no UI or Tauri dependency) |
| UI | Svelte 5 + TypeScript, built with Vite |
| Editor | CodeMirror 6 with `@codemirror/lang-markdown` |
| Preview rendering | markdown-it + DOMPurify |
| Graph view (v2) | Sigma.js (WebGL) + graphology, layout in a Web Worker |
| File watching | `notify` crate (desktop), rescans on resume (Android) |
| Sync server (v3) | Rust (axum + SQLite), shipped as a Docker image |
| Crypto (v3) | RustCrypto: XChaCha20-Poly1305, Argon2id |
| Tests | `cargo test` for core and sync, Vitest for UI logic, a small W3C WebDriver client + `tauri-driver` for desktop end-to-end, adb + the WebView DevTools protocol for Android |

### Why Tauri 2

Tauri 2 is the only option on the list that gives one codebase for Windows, macOS, Linux and Android while letting the heavy logic (indexing, search, sync, crypto) live in a compiled language. The UI is a web view, so the same Svelte code runs on all four platforms. Bundles are small (a few MB, against 80+ MB for Electron) and start quickly, which matters for the 2-second cold start target.

Rejected alternatives:

- Electron. Mature, with the best desktop support, but it does not run on Android at all. We would need a second app for v3, which breaks the one-codebase rule.
- Capacitor (+ Electron for desktop). Shares the web UI, but the core would be JavaScript, and on desktop Capacitor means Electron again. That is two shells and two native bridges.
- Flutter. Good on mobile and fine on desktop, but Flutter has no CodeMirror-class Markdown editor. We would have to write the editor, and the editor is the product.
- React Native (+ RN Windows/macOS). It does not support Linux, and the code editing components for RN are weak.
- Plain web app / PWA. No real file system access on Android or Firefox, so "files are the source of truth" fails.

The Tauri risk is Android: Tauri's mobile support is younger than its desktop support, and storage on Android needs native Kotlin code (see section 5). That platform code is small and sits behind one Rust trait.

### Why CodeMirror 6 for the editor

The file on disk is the document. CodeMirror edits plain text, so whatever the user types is exactly what gets saved, byte for byte. Obsidian's editor is also CodeMirror 6, and its Live Preview mode is built from CM6 decorations, so we know the approach works.

ProseMirror and Tiptap edit a rich-text tree and serialize it back to Markdown. That round trip is lossy: it normalizes list markers, emphasis style, spacing, tables and unknown syntax. Cairn's files are meant to be opened in other editors too, so silently rewriting the user's Markdown is not acceptable. Milkdown has the same problem.

Monaco is a code editor with heavy bundles and poor mobile support.

### Why Svelte 5

Svelte has a small runtime and fine-grained reactivity without a virtual DOM, starts fast, and keeps component code simple. React would also work, but Svelte's smaller bundle helps on Android web views. The UI is a thin layer, and nothing in the core depends on this choice.

### Why core logic in Rust

- One implementation of notebook operations, link parsing, indexing and sync for every platform, testable with `cargo test` without a UI.
- Fast enough to scan and index 10,000 notes well inside the start-up budget.
- The sync engine and the crypto must be identical on desktop and Android; sharing a Rust crate guarantees that.
- The sync server can reuse the protocol types.

The UI still does a small amount of Markdown parsing for editor features (syntax tree, live preview, autocomplete). That is presentation logic, not data logic.

## 2. Architecture

The code calls a notebook a vault: `Vault` and `VaultFs` in the core, the `vault://` protocol, and `/vaults/` in the sync API. Only the text users read says notebook.

```
┌──────────────────────── Svelte UI (TypeScript) ─────────────────────────┐
│ file tree │ tabs + CodeMirror editor │ preview │ backlinks │ switcher │
│                         src/lib/backend.ts (typed API)                  │
└───────────────────────────────┬─────────────────────────────────────────┘
                     Tauri invoke() / events ("vault-changed")
┌───────────────────────────────┴─────────────────────────────────────────┐
│ app/src-tauri: thin command layer, watcher thread, platform glue        │
├─────────────────────────────────────────────────────────────────────────┤
│ cairn-core                                                              │
│   VaultFs trait ── StdFs (desktop, Android app storage)                 │
│                └─ SafFs  (Android Storage Access Framework, v3)         │
│   Vault: file ops, path rules, atomic writes, self-write detection      │
│   Scanner: disk snapshot vs index → Created/Modified/Deleted/Renamed    │
│   Parser: wikilinks, md links, embeds, tags, frontmatter, headings      │
│   Index: notes, link resolution, backlinks, full-text search            │
├─────────────────────────────────────────────────────────────────────────┤
│ cairn-sync (v3): crypto, protocol, sync state, 3-way merge, conflicts   │
└─────────────────────────────────────────────────────────────────────────┘
                                 │ HTTPS
                     cairn-server (v3, Docker): encrypted blob store
```

### Folder structure

```
cairn/
  Cargo.toml                 Rust workspace
  crates/
    cairn-core/              notebook, fs abstraction, parser, index, search
    cairn-sync/              (v3) sync client engine, crypto, merge
    cairn-server/            (v3) sync server + Dockerfile
  app/
    package.json, vite.config.ts
    src/                     Svelte UI
      lib/backend.ts         the only module that talks to Tauri
      lib/editor/            CodeMirror extensions (wikilinks, live preview)
      lib/corePlugins/       core plugins: optional features with a switch in Settings
      lib/components/        tree, tabs, panels, dialogs
    src-tauri/               Tauri shell (Rust), commands, watcher
      gen/android/           (v3) generated Android project + Kotlin plugin
  e2e/                       end-to-end tests: a small WebDriver client + tauri-driver (desktop), adb (android/)
  scripts/                   notebook generator for perf tests, etc.
  docs/PLAN.md
  docs/RELEASE_NOTES.md
  README.md
  LICENSE
```

### Key design decisions for v3 readiness

1. File access goes through a `VaultFs` trait (list, read, write, rename, delete, mkdir, stat). Desktop uses `std::fs`. Android can use `std::fs` on app storage or a SAF implementation backed by Kotlin. Nothing above the trait knows which one it has.
2. Note identity is the path relative to the notebook folder, with `/` separators and NFC Unicode normalization. This is what other editors see, so it is the only identity that survives the user editing the folder by hand. Sync adds its own stable file IDs in its local state (never in the notes), and maps renames onto them.
3. Change detection is diff-based, not event-based. The index keeps `(path, mtime, size)` for every file and the content hash of every note. A scan compares disk with that snapshot and yields `Created / Modified / Deleted / Renamed` changes. A rename is a delete and a create with the same content (for an attachment: the same extension, size and mtime). When several files share that content, the scanner pairs files with the same name first, then files in the same folder, and a single pair left over is a rename too. An empty file never counts as renamed. The desktop file watcher only tells the scanner where to look. Android and sync use the same scanner and the same rename pairing. Missed watcher events do no harm because the next scan catches them.
4. Cairn recognizes its own writes by content hash. When it writes a file it updates the index first, so the watcher event that follows finds an identical hash and is ignored. An external edit shows up as a real change.
5. The core has no UI or Tauri types. Tests drive it directly.
6. App config that belongs to the notebook (settings, hotkeys, custom CSS, plugins) lives in `<notebook>/.cairn/`, like Obsidian's `.obsidian/`. Caches, sync state and plugin approvals live in the OS app-data and config directories, keyed by the notebook folder's path, so they are never synced and other tools never see them. The tree, the index and sync leave out dot-folders and dot-files, and the note commands and `vault://` refuse them.
7. Saves are atomic on desktop and in Android app storage (`StdFs`): Cairn writes a temp file with a random name that starts with a dot (`.cairn-tmp-`) in the same folder and renames it over the target, so a crash cannot leave a half-written note. Cairn does not list that file, but on Windows File Explorer shows it. A save that carries a base hash checks the file on disk again right before the rename and writes nothing if the file has changed; on Windows this last check compares only the size and the modification time (section 9). Cairn refuses to save over a read-only file. It writes a symlinked note at the link's target. A hard-linked note is written in place after a full temp copy, so the links stay, but that write is not atomic (section 9). On Windows, where the standard library cannot read a file's link count, a save replaces a hard-linked note and the link splits. SAF folders have no atomic replace (section 5).
8. Delete moves files to the OS trash on desktop (falling back to `<notebook>/.trash/`), and to `<notebook>/.trash/` on Android.
9. The name on disk can differ from the path in the notebook. `StdFs` maps each NFC path in the notebook to the real name, so files whose names are in NFD (as macOS writes them) open, save and rename in place. When one folder holds two names that are equal after NFC, Cairn lists the second as `name (Unicode twin).md` and renames nothing on disk. A name with a backslash (possible on Linux and Android) has no path in the notebook: Cairn does not show, index or sync it, and lists it with the files not synced. SAF folders match names in NFC and have no twin names.
10. The app, `vault://` and sync follow symlinks the user made. A folder link that leads back to a folder it is in (`loop -> .`) is not listed. A file that the notebook reaches under two names (a folder linked in twice, or a symlink to a note; a hard link is a file of its own) is listed under both and synced under one: the name a synced file has, else the one with no link on the way, then the one with the fewest folders, then the first in byte order. Sync applies no remote change through a name it does not sync (FINDING-224). A config file under `.cairn/` follows a link only to a target inside the notebook; a link that leads out is replaced by a plain file. If a folder on the way to a config file leads out of the notebook, the write fails with an error and writes nothing; reads still follow it. The plugin API refuses a path when a link on the way leads out of the notebook, into a hidden folder, or from a note to another kind of file.
11. Core plugins are optional features that are part of the app (`app/src/lib/corePlugins/`): Templates, Daily notes, Unique note creator and Random note. Each one has a switch under Settings, then Core plugins, and the app works the same with all of them off. They are app code, not sandboxed plugins, so they need no approval on each device: a notebook can only switch them on or off and set their options. They act only through their commands and buttons, never when a notebook opens. They reach the notebook through a small host interface in the app (`CoreHost`), and every note they make goes through the same create call as New note, which never writes over a file; text they insert goes into the editor like typing. Their switches and options live in `<notebook>/.cairn/settings.json` under `corePlugins`, holding only what the user changed (a missing value means the default). The object is kept as read, so plugin ids and options a version does not know, and values of the wrong type, stay in the file; version 1.0.0 also keeps the key when it saves other settings.
12. Themes are sets of CSS variables in `app/src/app.css`, listed in `app/src/lib/themes.ts`: Limestone, Marble and High contrast light are light. Slate, Graphite and High contrast dark are dark. Settings, then Appearance, lists them in one Theme list after System. The settings keep `theme` (system, light or dark) as before and add `lightTheme` and `darkTheme` with the id of the light and dark theme to use, written only once chosen. Picking a theme saves its scheme in `theme` and its id in `lightTheme` or `darkTheme`; System uses both, and two small lists under it change them. A new theme cannot go into `theme` itself, because 1.0.0 to 1.2.0 replace a value they do not know there with "system" on their next save; they keep keys they do not know. An id this version does not know shows the default theme and stays in the file. The app puts the ids on `<html>` as `data-light-theme` and `data-dark-theme`, and each theme's rule has the same CSS specificity as the Limestone or Slate rule it replaces, so System switches with the system in CSS alone and snippets override every theme the way they override those two. Each theme names the contrast its text keeps (`minContrast`): 4.5:1 (WCAG AA), or 7:1 (AAA) in the high-contrast themes. These also set `--ring`, the width of the rings around selected rows, pressed buttons and focused controls, which is 0 in the other themes; every theme sets it, so a light theme's value never shows in a dark one. A Vitest test reads `app.css` and checks every theme's text against the backgrounds it is drawn on at the theme's minimum, and its UI colors at 3:1, with and without a custom accent, which is adjusted to the theme's minimum.
13. A font file for note text lives in `<notebook>/.cairn/fonts/` and is named by `textFont` in the settings, so it moves with the folder as snippets do. `fontFamily` (sans, serif or mono) stays the Text font and the fallback, because 1.0.0 to 1.2.0 replace a value they do not know there with "sans"; they keep `textFont`. `textFont` is written only once a file is chosen and kept as read. The app reads the file with a command that returns raw bytes and loads it with the CSS Font Loading API (`new FontFace(name, bytes)`), which the CSP does not limit as it limits font URLs; `vault://` refuses dot-folders on purpose. The font loads with the notebook and needs no approval on each device: a snippet can already embed a font as a data: URL. Its name (woff2, woff, ttf or otf), its first bytes and its size (at most 20 MB) are checked before it is saved or loaded. Android passes no raw request body to a command, so the app sends the bytes there in base64. The file is found by the name in `textFont`, not by listing `.cairn/fonts/`. A file replaced by one of another name, or removed, in Settings goes to the trash; one of the same name, ignoring case, is written over, since macOS, Windows and Android shared folders take the two names for one file. Writes and moves to the trash never leave the notebook through a link, as for other config files, and the read refuses anything but a plain file and stops at the size limit.

### Link resolution rules (Obsidian compatible)

`[[target#heading|alias]]` resolves by: exact path in the notebook (`.md` or `.markdown` is optional), then a path relative to the linking note's folder, then a case-insensitive basename match. Link text is compared in NFC, and `\` is a folder separator like `/`. A target that starts with `./` or `../` resolves relative to the linking note's folder; if nothing is there, or `../` climbs out of the notebook folder, Cairn uses a note whose path ends in the remaining folders. When several notes share a basename, the one in the linking note's folder wins, then the shortest path (in characters), then code point order. Standard Markdown links to local `.md` files count as links too; they resolve first as an exact path relative to the linking note. Links inside code spans and code blocks are ignored, and so is a wikilink escaped as `\[[...]]`.

## 3. Sync strategy

### Options considered

| Option | Pros | Cons |
|---|---|---|
| Git (self-hosted remote) | History for free, familiar | Needs libgit2 on Android, merge conflicts written as markers into notes, no E2E encryption without git-crypt, repo grows forever with attachments |
| WebDAV (Nextcloud etc.) | Many users already have a server | No change feed (must list everything to find changes), no atomic compare-and-swap on most servers, E2E must be layered on top, slow on big notebooks |
| Syncthing-style P2P | No server | Devices must be online at the same time, NAT traversal, the Android Syncthing app was discontinued, conflict handling is file-level only |
| CRDT (Automerge / Yjs) | Automatic merging of concurrent edits | Files are the source of truth, so the CRDT is a shadow copy that must be reconciled with external edits anyway. Large state per note, complex, and still needs a relay server |
| User-hosted Cairn server | Change feed, atomic revisions, E2E by design, simple to run in Docker | One more thing to host; we have to write it |

### Decision: small user-hosted server, encrypted revisions, 3-way merge on the client

The server is a dumb store for encrypted blobs. It never sees paths or content.

- Each file has a random 128-bit `file_id` chosen by the device that first uploads it. The server keeps a revision log per file: `(file_id, rev, parent_rev, device, deleted, blob)` and a global, monotonic `seq` that every new revision gets.
- A blob is `XChaCha20-Poly1305(notebook_key, {path, content, mtime})` with a random nonce. Paths are inside the ciphertext, so renames are just new revisions. The server sees IDs, sizes and timestamps, and the name of the device that uploaded each revision, which is sent in plaintext.
- Keys: the user's passphrase goes through Argon2id (salt stored on the server) to a key-encryption key, which unwraps a random notebook key stored on the server in wrapped form. Changing the passphrase rewraps one key. A wrong passphrase fails the AEAD check, so the client detects it before it touches any file.
- Access to the server is a bearer token from the server's config (single user, many notebooks). A reverse proxy terminates TLS; the docs show Caddy.

Client loop (manual "Sync now" plus a timer when online):

1. Check the server. If it no longer has the notebook, or has fewer changes than this device has seen (a reset or a restored backup), stop with an error; the user turns sync off and connects again.
2. Scan the notebook (the same scanner as above) and compare with the local sync state, which stores for each `file_id`: path, last synced revision, and the content hash and text of the last synced version (the merge base). If the notebook folder has no visible files while the state tracks live files, stop and push nothing: a moved or unmounted folder looks the same.
3. Pull: fetch revisions with `seq > last_seq` in batches of about 64 MB, decrypt, and apply each batch, deletions first, before reading the next. Write remote changes directly to files the user has not touched, after a last check on disk, so a note saved during the sync is kept. Skip a record that cannot be decrypted. A change this device cannot apply (a name its file system refuses, for example one that differs only in case from a name already in an Android shared folder; a file where a folder is; a folder it cannot read; a second copy of a file that this device syncs under another name, through a symlink) stays pending and is retried on every sync.
4. Push: upload local changes with `parent_rev = last synced rev`. The server accepts only if `parent_rev` is still the head (compare-and-swap). If it is not, another device got there first, so pull and merge, then retry. An upload that fails on its own (too large, stalled) is retried on the next sync. The app lists every file a sync left out, with the reason.

If the sync state is missing or damaged, the client rebuilds it before the push by matching local files to server files by path and content, so no note is uploaded twice. A sync that rebuilds the state deletes nothing.

Conflict rules, chosen so nothing is lost silently:

- Both sides edited a Markdown file: run a 3-way merge (base, local, remote) with `diffy`. If it is clean, keep the merge. If the changes touch the same or neighboring lines, or either side rewrote more than about 10,000 lines since the last sync (more than 20,000 lines added and removed; lines only added or only removed in one place do not count), keep the local file as is and write the remote version next to it as `Note (conflict 2026-10-03 1530 Pixel).md` (the date and time in UTC, and the device that made the copy). The sync status UI marks the conflict copy. Conflict copy names fit every file system.
- Binary attachments edited on both sides: keep both (conflict copy).
- Edit vs delete: the edit wins and the file is restored. Deleting is easy to redo; losing an edit is not.
- Rename on one side, edit on the other: both are applied, because the `file_id` links them. Sync knows a rename made in Cairn while sync is set up, even when the note is edited before the next sync. Sync finds a rename made outside Cairn by the note's unchanged content (decision 3 in section 2); otherwise the rename counts as a delete and a new file.
- Rename on one side, delete on the other: the rename wins, whichever device syncs first, and the file keeps its `file_id` (the renaming device uploads the rename on top of the delete). A folder rename counts as a rename of each file in it, so a note deleted on one device inside a folder renamed on another is kept in the renamed folder.
- Rename on both sides to different names: the name already on the server (from the device that synced first) wins; content is merged as above.
- Two devices create the same path independently: they have different `file_id`s. If the contents differ, the file uploaded first gets a conflict copy name on every device and the other keeps the name. Identical files become one note. When both were uploaded before either device saw the other's, a device that sees both keeps the later-uploaded `file_id` and deletes the earlier one on the server, checked against its head. If either copy changed since that device's pull, it deletes nothing, and the earlier one becomes an ordinary conflict copy. A device that holds the deleted copy unchanged keeps its file under the kept `file_id`, and the server keeps the deleted one's history.
- On a file system that ignores case (macOS, Windows), a remote file whose name differs only in case from a local one gets a conflict copy name on every device. Android shared storage refuses such a file: the change stays pending, is retried on every sync and is listed with the files not synced, so no device renames it. A remote rename to such a name waits the same way, together with any edits made to the note on the phone in the meantime. If the user renamed the note on the phone too, the user's name wins. When the phone already has a copy of a remote note, with the same content and a name that differs only in case, it gives its copy the remote name and uploads no rename (FINDING-172).
- One file under two names on a device (a folder linked in twice, or a symlink to a note) syncs under one name there. A second copy that an older version uploaded under the other name stays on the other devices. The device with the link sends nothing for it and does not apply its changes: a delete of it, or a version with the note's current content (as a device on an older version uploads whenever the note changes), is recorded, and any other edit or rename waits and is listed. When another device deletes the synced copy and keeps the second one, the note stays, and the second copy syncs in its place. Deleting the note on the device with the link deletes both copies, unless another device changed the second copy meanwhile (edit beats delete) or the link was removed before (FINDING-224).

Version history: the server keeps every revision (there is no pruning yet), which makes "restore an older version" possible.

## 4. Search and index

The index lives in memory, and Cairn rebuilds it from the files on every start. Reading, hashing, parsing and tokenizing run in parallel (rayon); inserting into the index is sequential. The full-text index is an inverted index (term to note postings) with prefix matching, phrase checks, `tag:` and `path:` filters (quoted when the value has spaces: `path:"My Folder"`), BM25 scoring and a boost for file-name matches. Updates are incremental per file: the watcher and every write re-index only the files involved.

Search compares text after NFC normalization and lowercasing; accents still count. Chinese and Japanese text is indexed as overlapping pairs of characters. Thai, Lao, Khmer and Burmese text is indexed one run of letters at a time, so search cannot find a word inside a sentence in those scripts.

The plan originally included a persisted index snapshot for fast warm starts. Measurements in v2 showed it is not needed: a 10,000-note notebook (14.5 MB) indexes in about 130 ms, so we dropped the snapshot. That leaves one less cache that could go stale. A 10,000-note notebook of Chinese and Japanese text (2.9 million postings) takes about 340 ms.

## 5. Android storage (v3)

Android gives three kinds of storage:

1. App-specific storage: works with `std::fs`, needs no permission, and most other apps cannot see it. It is the default for a new notebook on Android.
2. A folder picked with the Storage Access Framework (`ACTION_OPEN_DOCUMENT_TREE` with a persisted URI permission). This is the correct, Play-Store-friendly way to use a shared folder, but it only exposes `content://` URIs, not paths. Cairn implements `SafFs` (the `VaultFs` trait) over a Kotlin Tauri plugin that uses `DocumentsContract` queries directly (DocumentFile is too slow for large trees). There is no inotify here, so Cairn picks up changes with a rescan when the app resumes and on a timer.
3. All-files access (`MANAGE_EXTERNAL_STORAGE`), which allows `std::fs` on shared storage. Cairn does not implement it: SAF already covers shared folders without a broad permission, and Play only allows all-files access for specific app categories.

Platform-specific code, all of it small:

- `app/src-tauri/gen/android/.../SafPlugin.kt`: the SAF bridge (folder picker with persisted permission, a check that the permission and the folder are still there, list, stat, read, write, mkdirs, move, remove empty folder). It runs on a background thread and caches path-to-document-id lookups. Shared storage ignores case, so the plugin matches names in NFC, ignoring case, and refuses names the storage cannot hold and names that differ only in case from an existing one. A missing tree root is an error, not an empty folder. The plugin writes a new file to a temp document and renames it into place; it overwrites an existing file and then cuts it to its new length.
- `app/src-tauri/src/android.rs`: `SafFs`, the `VaultFs` implementation that calls the Kotlin plugin. Deletes move files into the notebook's `.trash/`. Like `StdFs`, a listing leaves out names that start with a dot below the folder it lists, so the tree skips `.cairn` while Settings lists the snippets and plugins in it.
- `MainActivity.kt`: pads the web view by the system bar and keyboard insets (Android 15 draws apps edge to edge, and the web view does not see those insets as CSS safe areas). It passes Back to the page, which closes the top overlay, or saves and leaves the app when none is open. When the web view's renderer crashes, it recreates the activity and the page reopens the notebook and the last note; this needs a web view client override in `app/.cargo/config.toml`, so cargo must run inside `app/`, as `tauri android build` and Gradle do.
- In Rust, the file watcher and system trash are compiled for desktop only (`cfg(desktop)`); Android rescans when the app returns to the foreground and every 20 seconds while visible. At start the app reopens the last SAF notebook only if it still has the permission and the folder exists.
- In the UI, `platform.ts` switches to the narrow layout (sidebars as drawers, a top bar, a formatting toolbar above the keyboard, long-press menus with "Move to…").

## 6. Risks

| Risk | Mitigation |
|---|---|
| Tauri Android maturity, SAF performance on big notebooks | `VaultFs` keeps SAF isolated; app storage is the default; cache the listing; measure early in v3 |
| Android toolchain setup | The README lists the SDK, NDK and JDK versions, `scripts/android-env.sh` finds them, and the Android tests run on the emulator |
| Live Preview editing is subtle (cursor, selection, widgets) | Build on CM6 decorations like Obsidian; keep a source-mode toggle; cover the tricky cases with tests |
| Case-insensitive file systems (macOS, Windows, Android shared storage) vs case-sensitive Linux | Refuse new case twins; link resolution is case-insensitive; sync gives a pulled case twin a conflict copy name on macOS and Windows, and Android shared storage refuses it and lists it with the files not synced |
| Watcher floods (git checkout, bulk copies) | Debounce and coalesce, then run one diff scan; an inotify queue overflow triggers one full rescan |
| Sync data loss | Compare-and-swap revisions, merge base kept locally, conflict copies, no automatic deletes on conflict, two-device simulation tests; sync stops when the notebook folder is empty or the server has lost changes |
| E2E key loss | A clear warning in the UI that a lost passphrase means lost data. The server cannot recover it |
| Graph performance at thousands of nodes | WebGL renderer (Sigma), layout in a worker, render labels only when zoomed in; still slow at 50,000 notes (section 9) |
| WebKitGTK quirks on Linux | E2E tests run on WebKitGTK itself via `tauri-driver` |

## 7. Milestones

### v1 (usable desktop app)

1. Repo scaffold: Cargo workspace, Tauri 2 + Svelte app, CI-style scripts.
2. `cairn-core`: `VaultFs`, `Vault` ops (create/rename/move/delete files and folders, atomic write), path rules, scanner, parser (wikilinks, md links, headings), index with link resolution, backlinks and basic full-text search. Unit tests.
3. Tauri commands and the watcher thread emitting `vault-changed`.
4. UI: notebook picker and recent notebooks, file tree (expand/collapse, context menu, drag-and-drop move), tabs, CodeMirror editor with autosave, preview toggle and split view, wikilink autocomplete, clickable links, create-on-click for missing notes, backlinks panel, quick switcher (Ctrl/Cmd+O), search panel.
5. E2E smoke tests with `tauri-driver`. Packaged Linux build (deb, rpm, AppImage).

### v2 (feels good)

1. Live Preview in CodeMirror (inline rendering, hide syntax off the cursor line, widgets for images, checkboxes, embeds), source-mode toggle.
2. Persisted index snapshot, incremental re-index, perf harness with a 10k+ note generated notebook, cold start < 2 s.
3. Graph view (Sigma + worker layout).
4. Tags, frontmatter properties panel, `![[note]]` embeds.
5. Attachments: paste and drag-drop into `attachments/` (configurable), image rendering.
6. Themes (light, dark, custom CSS from `.cairn/snippets`), settings screen.
7. Command palette and customizable hotkeys.

### v3 (multi-device)

1. Android build from the same codebase, touch layout, SAF plugin.
2. `cairn-sync` + `cairn-server` with E2E encryption, Docker image, setup docs.
3. Sync status UI, "Sync now", conflict list.
4. Two-device concurrency tests (in-process server, two notebooks).
5. Stretch: version history and restore; sandboxed plugin API.

## 8. Status

v1, v2 and v3 below are development milestones. Version 1.0.0, the first public release, is the state after the fixes at the end of this section.

### v1: done

Verified on Arch Linux (GNOME, Wayland, WebKitGTK 2.52):

- `cargo test -p cairn-core`: 44 tests (paths, file system, parser, link resolution, backlinks, search, notebook operations, change detection including external renames of files and folders).
- `npm test` in `app/`: 13 tests (tree building, link helpers, Markdown rendering).
- `npm run e2e`: 17 end-to-end tests that drive the built app through `tauri-driver` and check the files on disk. They cover the tree, opening notes, autosave, backlinks, `[[` autocomplete, Ctrl+click on links, preview and creating a note from a broken link, the quick switcher (open and create), search snippets, new note with inline rename, new folder, drag-and-drop move, delete with confirmation, external create/modify/delete/rename, and the conflict banner.
- `npx tauri build` produced `.deb`, `.rpm` and AppImage packages. The AppImage and the release binary were started against a sample notebook and opened it.

Known gaps and limits at the end of v1:

- Only the Linux packages were built and run. Nothing in the code is Linux-specific (the watcher and trash crates support macOS and Windows), but macOS and Windows builds are untested.
- WebDriver cannot drive the native folder picker, so the automated tests do not cover the "Open folder as notebook" and "Create new notebook" buttons. They do cover the typed-path field next to them.
- Attachments are listed in the tree but cannot be opened or displayed yet. `![[embeds]]` render as plain links. Both are v2.
- Performance on large notebooks was not measured. The index is built synchronously on open, and every save refetches the full file list. v2 addresses both.
- Renaming a note does not rewrite links that point to it.

### v2: done

What was added, all verified on the same Linux machine:

- Live Preview editing in CodeMirror: syntax is hidden on lines without the cursor; headings, emphasis, inline code, links and wikilinks (aliases shown), tags, quotes, bullets, clickable checkboxes, rules, tables, images, audio/video, note embeds and frontmatter (as a properties box) render inline. Source mode, reading view and split view remain, per tab.
- `vault://` protocol that serves notebook files to the web view through `VaultFs` (so it will also work for Android SAF notebooks).
- Embeds: `![[note]]`, `![[note#heading]]`, `![[note#^block]]`, nested up to 3 levels with cycle protection; image embeds with sizes (`![[pic.png|200]]`); relative Markdown image paths.
- Attachments: paste or drop files into the editor, and Cairn saves them to the attachment folder (setting, default `attachments/`) under a free name and links them. Attachments in the tree open in the system's default app if their type is on the allowed list (section 9).
- Tags panel with counts, `tag:` and `path:` search filters, outline panel, properties panel.
- Graph view (Sigma.js WebGL + ForceAtlas2 in a Web Worker): hover highlights neighbors, click opens, find a node, optional unresolved nodes, updates when links change.
- Settings stored in `<notebook>/.cairn/settings.json`: theme (system, light, dark), accent color, text font, font size, line width, default view mode, spell check, attachment folder, CSS snippets from `.cairn/snippets/` (created and edited in the settings screen).
- Command registry with about 35 commands, a command palette (Ctrl+P) and customizable hotkeys (record, remove, reset; conflicts are resolved and reported). Editing commands: bold, italic, inline code, strikethrough, checkbox cycling, insert link.

Measurements (release build, Ryzen desktop, NVMe, generated notebook from `scripts/gen-vault.mjs`, 10,000 notes, 14.5 MB):

| What | Result |
|---|---|
| Index build on open | 127 ms (was 339 ms single-threaded) |
| Process start to first painted UI | about 400 ms (target: under 2 s) |
| Search ("garden", "river stone", prefix "harv") | 2 to 3 ms |
| Graph data (10,000 nodes, 43,331 edges) | 16 ms |
| Graph frame interval during layout | median 6 ms, p95 9 ms, one 54 ms frame |
| Typing 226 characters mid-way through a 3,000-line note | no frame above 50 ms; worst 34 ms (Live Preview), 28 ms (source) |

Cold-start numbers were taken after evicting the notebook from the page cache with `scripts/evict-cache.py` (`posix_fadvise`); they matched the warm numbers on this NVMe drive, so a slow disk will be slower than this.

Tests: 37 Rust tests in `cairn-core`, 17 Vitest tests, and 28 end-to-end tests (the v1 set plus Live Preview widgets, checkbox toggling, link clicks, command palette, formatting hotkeys, rebinding a hotkey, graph contents, theme and CSS snippet, tags panel and tag search, outline and properties, and dropping a file into the editor).

Known gaps at the end of v2:

- Live Preview reveals syntax per line (the whole line under the cursor), where Obsidian reveals per element. Callouts, math, footnotes and raw HTML blocks are not rendered in Live Preview.
- The properties panel is read-only; properties are edited as YAML in the note.
- Renaming a note still does not rewrite links that point to it.
- Embedded notes refresh when the embedding note re-renders, not immediately when the embedded file changes.
- The graph is global only (no local graph or filters beyond unresolved nodes).
- The automated tests dropped files with a synthetic drop event. They did not exercise a real drag from the OS file manager or pasting from the clipboard.
- macOS and Windows remain untested.

### v3: done

Sync (verified):

- `cairn-server`: axum + SQLite, bearer tokens, changes feed, compare-and-swap uploads, revision history. Built as a Docker image (multi-stage, non-root, health check) with a Compose file that puts Caddy in front for HTTPS. The image was built and run here with rootless Docker, and the `smoke` example synced two notebooks through the container.
- `cairn-sync`: Argon2id key derivation, a wrapped random notebook key, XChaCha20-Poly1305 per revision with the file id as associated data. Paths are inside the ciphertext. A test reads the server's SQLite file and checks that no note text or file name appears in it.
- Merge rules as in section 3: three-way merge of notes, conflict copies for overlapping edits and for attachments, edit beats delete in both directions, rename and edit combine, folder renames, both sides creating the same path, deletes go to the trash.
- Tests: 20 two-device tests through a real server over HTTP, including a forced upload race (a 409 followed by a merge in a second round) and a randomized test (200 random edits, creates, deletes, renames and syncs per run on two devices) that checks the devices converge and that every line ever written survives in a file or a trash. It passed 60 seeds in a row.
- In the app: a background sync thread (4 seconds after the last local change, every minute, and on demand), the status in the status bar, a setup screen, a list of conflict copies, and version history with restore. 6 end-to-end tests run the real app against a real server with a second device driven by the `sync_dir` example, including a merge triggered by typing and a conflict copy.

Android (verified on the Android 15 x86_64 emulator, debug and release builds):

- The same Svelte UI and Rust core; Android-only code is listed in section 5.
- 7 end-to-end tests drive the app through adb and the WebView's DevTools protocol: creating a notebook in app storage with real taps, typing with the device keyboard (checked in the app's private storage), Live Preview, the drawer and the long-press menu, picking up a file added while the app was in the background, two-way sync with a desktop device through a server on the host, and opening a shared folder through the system picker, editing a note there and creating a new one.
- A signed (test key) release APK with R8 minification was installed and opened an SAF folder, which checks that the reflectively loaded plugin survives minification.

Plugins (stretch goal, done): JavaScript files in `.cairn/plugins/`, each in its own Web Worker. The API offers commands and toasts. It also offers reading notes, writing notes and the editor selection if the file header declares them and the user confirms them when enabling the plugin. An end-to-end test runs a plugin that tries to reach the DOM, the Tauri bridge, the network and the IPC endpoint directly, and to read a note without permission; all of those fail, and the permitted editor call works.

Version history (stretch goal, done): the server keeps every revision; the app lists them per note and restores one as a new revision.

Known gaps at the end of v3:

- Only the x86_64 emulator was tested. The arm64 build compiles and is in the release APK, but no physical phone was available.
- The release APK is signed only with a throwaway test key; signing for distribution is up to the publisher.
- Sync does not cover the `.cairn/` folder (settings, snippets, plugins) or empty folders.
- The server can withhold or roll back revisions (it cannot read or forge them). Clients do not detect a rollback.
- The notebook key is stored unencrypted in the app's data folder on each device (the same place the notes are). The OS keychain would be better.
- On Android, SAF folders are slower than app storage for large notebooks, writes there are not atomic, and attachments cannot be opened in other apps yet.
- iOS was not attempted.
- macOS and Windows are still untested.

### Fixes before 1.0.0: done

Adversarial testing before 1.0.0 led to the fixes below. Section 9 lists the findings that are not fixed, with their FINDING numbers, among them FINDING-225 (a bug in @lezer/markdown). FINDING-224 (a file reached under two names through links) is fixed in 1.2.0, and section 9 lists what still holds of it. The rules that came out of this testing are in sections 2, 3 and 9, and the tests named after each finding check them.

- Edits that cannot be saved stay in their tab, and Cairn asks before it closes that tab, the notebook or the window. Closing the window saves first, and so does a SIGTERM to the app. Saves check the file again just before they replace it and keep symlinks and hard links.
- One bad record or file does not stop a sync, and the app lists the files left out. Sync stops when the notebook folder looks empty or the server has lost changes. A rename beats a delete in either sync order, and identical notes uploaded at once become one. A rename made in Cairn keeps the note's history even when the note is edited before the next sync, and a pull applies about 64 MB of changes at a time.
- Plugins need an approval on each device and can reach only the notebook's notes. Only listed file types open with the system's default app.
- Wikilinks with `./` and `../` resolve from the linking note's folder, `.markdown` notes and link text in NFD resolve, and an escaped `\[[link]]` is not a link. Search finds Chinese and Japanese words inside sentences and takes quoted filter values such as `path:"My Folder"`.
- On Android, writes to shared folders never empty a note, a shared-folder notebook reopens after a restart, Back closes overlays and saves, and the app reloads after a renderer crash.
- Typing in Live Preview in a 20,000-line note takes a median of 13 to 14 ms per key, and a graph hover on 10,000 notes about 45 ms, on one debug build. In the sync tests, the first upload of 5,000 notes takes about 5.3 s, and the time grows in line with the number of notes.
- Dialogs keep focus and give it back, the tree, tabs, menus and graph work from the keyboard, text meets WCAG AA contrast, and Ctrl+= and Ctrl+- zoom the window.

Tests for 1.0.0: 746 Rust tests (745 in `cairn-core`, `cairn-sync` and `cairn-server`, run with `CAIRN_FUZZ_SEEDS=200`, plus 1 unit test in the app crate), 232 Vitest tests, 476 desktop end-to-end tests and 85 Android end-to-end tests pass. Reproductions of findings that are not fixed stay in the suites, marked ignored, todo or expected to fail, with their finding id; the performance and memory measurements run only when an environment variable turns them on.

### After 1.0.0: core plugins (1.1.0)

- Templates (on by default): inserts a note from the template folder at the cursor, with `{{title}}`, `{{date}}`, `{{time}}`, `{{date:FORMAT}}` and `{{time:FORMAT}}` filled in, as one undoable edit. On a phone, a button in the formatting toolbar runs it.
- Daily notes (on by default): opens today's note, named by the date (`YYYY-MM-DD` by default) in a chosen folder (the notebook folder by default), and creates it from an optional template when there is none. A button next to Graph view runs it, in the Files drawer on a phone.
- Unique note creator (off by default): creates a note named by the date and time (`YYYYMMDDHHmm` by default); a taken name gets a number.
- Random note (off by default): opens a note picked at random.
- Dates use the moment.js format letters that Obsidian uses, with English day and month names on every device. Settings shows what a format gives today, or why it cannot make a file name.
- Commands of a plugin that is off are not in the palette or the Hotkeys list and their keys do nothing, but their hotkeys stay saved. None has a default hotkey.
- The status bar's word count also counts characters and the selection. Chinese and Japanese count one word per Han, Hiragana or Katakana character; Thai, Lao, Khmer and Myanmar are split with `Intl.Segmenter`.

Tests: 83 new Vitest tests (date formats and week numbers, template filling, note names the core would refuse, name clashes, each plugin's command against a stand-in for the app, word and character counts in Latin, Chinese, Japanese, Korean, Thai, Lao, Hindi, emoji and combining marks, and a `settings.json` with `corePlugins` read and saved by the settings code), 26 desktop end-to-end tests in 6 new files and 4 Android end-to-end tests in a new file. Each commit of the work passes its own tests. The existing end-to-end tests that use Settings, the command palette, the left sidebar, the status bar or the phone's formatting toolbar still pass.

### After 1.1.0: image tabs, themes and files under two names (1.2.0)

- Images (png, jpg, jpeg, gif, webp, avif, svg, bmp and ico) open in a tab inside Cairn, on the desktop and on Android: from the file tree, the quick switcher, search, links, the outgoing links panel and images embedded in a note. On the desktop, other files still open in the system's default app, and an image tab has Open in default app. On Android, a tap on another file still shows a message.
- The tab loads the image through `vault://` into an `<img>` only, so an SVG runs none of its scripts. The image's URL changes with each change to the file on disk, because the web view keeps the images it has shown in memory by URL. The tab follows renames and moves and closes when the file is deleted. An image replaces the active tab only when that tab shows an image too.
- Search lists images whose path holds every word and `path:` filter of the query, under the notes. The core's full-text search still covers notes only. The saved session leaves image tabs out, because 1.0.0 and 1.1.0 open every saved tab that is not the graph as a note.
- Two new themes, Marble (light: white page, blue-gray sidebars, navy text, a blue accent) and Graphite (dark: neutral gray with no tint, the rust accent), and names for the two that were there: Limestone and Slate. Settings, then Appearance, shows each light and dark theme with a small swatch. System uses the chosen light and dark theme.
- Colors that were below WCAG AA in code blocks, search results, hovered rows, under matches of the selected word and in the sync error in Settings were changed in Limestone and Slate. A custom accent is now also adjusted for hovered rows, code and those matches.
- The graph takes new colors as soon as the theme, the accent or a snippet changes; before, it kept the old ones until the next hover or reload.
- The editor's search (Ctrl+F) highlights matches in the theme's highlight color, the current match in full and the others at 70%. Before, CodeMirror's own yellow and orange outranked the rules in `app.css` for this, and some text on them was below 4.5:1. For this, Limestone's highlight is a little lighter and Slate's a little darker.
- A file that the notebook reaches under two names through symlinks (a link to a folder of the notebook, two links to one folder, or a symlink to a note) syncs under one name, and sync applies no remote change through the other name. Copies that older versions uploaded under the other name stay on the other devices. The device with the link sends nothing for them except their delete when the note is deleted there, and a copy that another device changed meanwhile stays. It applies no change to them from another device: it records a delete or a version with the note's content, and holds and lists any other change. When another device deletes the note's own copy and keeps the other one, that copy syncs in its place. Hard links sync as separate notes, as before.
- A remote move of a synced symlink to a file into another folder waits, listed, when the link's target is a relative path. An emptied folder link is not removed, and a folder that sync cannot remove after a remote change no longer holds that change back.

Tests: for image tabs, 66 new Vitest tests (where each file type opens on the desktop and on Android, which tab an image opens in, the tab after changes on disk, image search, and the saved session), a new desktop end-to-end file with 17 tests and a new Android end-to-end file with 2 tests; the end-to-end test for attachments in the file tree now also checks that an image opens in a tab. For the themes, 42 new Vitest tests (the contrast of every theme's text and UI colors against each background they are drawn on, also with 512 custom accents per theme; eight sample accents in Marble and Graphite; the swatches against `app.css`; Graphite free of tint; the theme keys read, saved, and kept when unknown), a new desktop end-to-end file with 8 tests (choosing each theme, the graph's colors after a theme, snippet or accent change, a restart, System on a light and on a dark system, an unknown theme id, a custom accent in each theme, the editor's search highlight in each theme, and the Settings layout at 375 and 320 CSS px), and 4 new tests in the contrast audits in the real app, which cover Marble and Graphite and now also show a code block, search results and the editor's search. The restore tests in `session_compat.test.ts` and the tests in `settings_compat.test.ts` also pass against the code of 1.0.0 and 1.1.0. For files under two names, 49 more Rust tests: `adv_dupe_links.rs` now has 48 sync tests in place of its 2 ignored reproductions of FINDING-224, and `cairn-core` has 3 new unit tests (the other names of a file, also in a folder that cannot be read, and an emptied folder link). The end-to-end tests that use the theme, the Appearance settings, the graph, the contrast audits or attachments in the file tree still pass.

Tests for 1.2.0: 793 Rust tests (792 in `cairn-core`, `cairn-sync` and `cairn-server`, also run with `CAIRN_FUZZ_SEEDS=200`, plus 1 unit test in the app crate), 423 Vitest tests, 522 desktop end-to-end tests and 91 Android end-to-end tests pass. The others are 36 Rust tests marked ignored (18 reproductions, 7 tests that pass and show behavior by design, and 11 slow measurements and probes), the reproductions marked todo (12 desktop tests) or expected to fail (1 Vitest test), 12 desktop tests that run only when an environment variable turns them on (performance, memory, a 50 MB note and slow plugin checks), and 3 desktop tests that skip when the title bar's close button cannot be reached through the accessibility bus.

### After 1.2.0: notebooks, Windows, the Theme list, high-contrast themes and a font file (1.3.0)

- The app says notebook where it said vault in all the text users read: the Welcome screen, dialogs, Settings, toasts, the command palette ("Switch notebook"), and the errors of the core, sync and the app shell. The sync server's log says "created notebook". The top of a notebook is the notebook folder ("Notebook folder" in the folder list of Move to and in the Folder option of Daily notes and Unique note creator). Identifiers, test ids, the command id `app:close-vault`, `CAIRN_VAULT`, the `vault://` scheme, the sync API with its error bodies, the settings keys and the file formats keep their names. Cairn shows the server's "no such vault" and "vault exists" as "no such notebook" and "a notebook with this name already exists".
- The card of an embedded file that does not show as text says what a click on its name does. On the desktop, Cairn asks a new command, `open_externally_check`, which runs the checks of Open in default app without opening anything: the card says the file opens in another app, or that Cairn does not open its type, a link to such a type or a text file marked as executable. When the check fails, the card names only the type. On Android, the card says that Cairn cannot open the file in another app yet.
- On Android, pasting or dropping a file into a note saves it: `save_attachment` now also takes the bytes as base64 in a JSON body, because Android passes no raw request body to a command.
- On Android, a folder opened from storage lists `.cairn/snippets/` and `.cairn/plugins/`. Its listing hid every entry under a dot-folder, even inside `.cairn` itself; it now hides an entry only when a name in its path below the folder listed starts with a dot, as `StdFs` does.
- CI (GitHub Actions) also builds and tests on Windows, with the MSVC toolchain: the Rust tests of `cairn-core`, `cairn-sync` and `cairn-server`, the unit tests of the app crate, the frontend build and Vitest, and clippy with warnings as errors. A second job builds the NSIS installer and keeps it for 7 days, and a release attaches the installer that CI built from the release's commit. CI does not run the window or the end-to-end tests on Windows, and nobody has tried the app by hand there. Section 9 lists the tests that do not run on Windows and what is known not to work there.
- What the Windows runs found and fixed:
  - On Windows, a name in a path in the notebook that starts with a drive letter and a colon ("D: plan.md", which sync can bring from another device) made Cairn read, write, rename or delete outside the notebook, on that drive. A colon later in a name made Cairn use a stream of another file in the notebook. Cairn on Windows now refuses any name that no Windows file can have, leaves such a name made on another system out of its listings, and makes no new file or folder whose name ends in a dot or a space or is a device name. Sync lists such a file from another device under "Files not synced". With the system trash, an entry whose path holds a name that Windows refuses goes to the notebook's `.trash` folder, because the system trash would take another file for it.
  - Cairn refuses device names (CON, PRN, AUX, NUL, COM0 to COM9 and LPT0 to LPT9, also with ¹, ² or ³, CONIN$ and CONOUT$, with any extension) for new notes, folders and attachments and for renames and moves made in Cairn, on every platform, as it refuses the characters Windows does not allow. Sync still applies such a name from another device, except on Windows, where it lists the file under "Files not synced".
  - On a file system that ignores case, the error for a new name that differs only in case from another entry now names that entry, as on Linux.
  - On Windows the app shows and keeps a notebook folder on a drive without the `\\?\` prefix, and gives the shell plain paths to open. A path keeps the prefix when it is longer than about 260 characters, or has a name that ends in a dot or a space or is CON, PRN, AUX, NUL, COM1 to COM9 or LPT1 to LPT9, alone or with one extension. A folder on a network share keeps its `\\?\UNC\` prefix (section 9).
  - On Windows the journal of renames made in the app could not be locked, so a rename could miss the next sync's list of renames.
- Settings, then Appearance, has one Theme list in place of the Theme choice and the Light theme and Dark theme rows: System, then every theme by name, grouped by light and dark, with a swatch of the theme in use. With System, two small lists under it pick the light and the dark theme. Picking a theme saves its scheme in `theme` and its id in `lightTheme` or `darkTheme`, the keys of 1.2.0, where the Theme choice and the theme rows set them separately.
- Two high-contrast themes, High contrast light and High contrast dark, keep all text at 7:1 or more (WCAG AAA) against every background it is drawn on, and borders and the scrollbar at 3:1. Only disabled buttons and a file tree row while it is dragged are dimmed below 7:1. Rings and bars in the accent show selected rows (command palette, quick switcher, version history, menus, autocomplete, the open note in the file tree and the Settings section shown), pressed panel buttons and focused controls. A custom accent is adjusted to 7:1 in them.
- In every theme, CodeMirror's matching bracket, bracket with no match, special characters and drop cursor, and the graph's hover box, take the theme's colors. For the outline of a matching bracket, Slate's input borders are a little lighter. Links to missing notes in the outgoing links panel are dashed, and a focused text field in Settings shows the accent border.
- A font file for note text (woff2, woff, ttf or otf, up to 20 MB) can be picked under Settings, then Appearance, then Font file. It is saved in `.cairn/fonts/` and named by `textFont`, and the Text font is used wherever it cannot be. New commands read and write binary files in `.cairn/` and move one to the trash. Cairn now refuses the settings, a snippet or a plugin in `.cairn/` that is larger than 16 MB, and except in an Android shared folder it refuses a link there to a device or a pipe, which it used to read without end.
- The desktop end-to-end tests no longer hang after their last test when WebKitWebDriver stops answering: closing a session and taking a screenshot give up after 15 s, and node lets go of tauri-driver's output once tauri-driver exits. Two tests of the conflict banner wait for it to go instead of checking once.

Tests: for the word notebook, 3 Vitest tests (a scan of the string literals in the app's Svelte, TypeScript, Rust and Kotlin code and in the Android resources that fails on the word vault outside a list of allowed names, and checks that the list holds no name that is gone), 1 Rust test (the server's errors "no such vault" and "vault exists" shown with the word notebook) and a new desktop end-to-end file with 2 tests (the Welcome screen, the status bar and every Settings section). For the embed card, 7 Vitest tests (the card for each answer of the check on the desktop, and on Android), 2 unit tests in the app crate (the checks of Open in default app, one of them on Unix only) and 1 desktop end-to-end test (a program, a file with no extension, an executable log, a link to a program and a PDF, in Live Preview and in the reading view); the two Android end-to-end tests of image tabs now also check the card. For pasting and dropping on Android, 2 Vitest tests, 1 unit test in the app crate and a new Android end-to-end file with 2 tests (in app storage and in a shared folder: every byte value, a name with `#` and accents, a taken name). For the shared-folder listing, 2 unit tests in the app crate (the listing compared with `StdFs`) and a new Android end-to-end file with 10 tests (five checks in a shared folder and five in app storage: the listing, a snippet turned on in `.cairn/settings.json`, a snippet turned on in Settings, a snippet made in Settings and kept after a restart, and a plugin turned on and run). For Windows, 4 Rust tests in the three crates (names that Windows refuses, in the path rules, in `StdFs` and in sync, and device names) and 1 in the app crate (the notebook folder without `\\?\`); the other changes made tests portable or limited them to the systems they need. For the Theme list, 7 new Vitest tests (what the list shows for the saved settings, what a pick saves, and which theme is on screen). The desktop end-to-end file for themes now checks the list, the Light and Dark lists under System, a theme of a later version, and the layout at full width and at 375 and 320 CSS px. Seven other end-to-end files that pick light, dark or System do it through one helper. For the high-contrast themes and the colors outside the themes, 28 new Vitest tests (both themes at 7:1 for text and 3:1 for borders, the scrollbar and the rings, also with 512 custom accents each; eight sample accents in each; every theme setting the ring width and color; the graph's hover box), 2 new desktop end-to-end tests (the rings, in the high-contrast themes only, and the bracket, special character, drop cursor and hover box colors in all six themes) and 4 new tests in the contrast audits in the real app, which hold the high-contrast themes to 7:1. For the font file, 16 new Vitest tests (file names, first bytes and size, loading and falling back, a load that finishes too late, and `textFont` kept by this version and by 1.0.0, 1.1.0 and 1.2.0), 4 new Rust tests in `cairn-core` (bytes in and out of `.cairn/`, the size limit, the trash, links out of the notebook and a link to a device, the last two on Unix only) and 1 unit test in the app crate (the command's raw and base64 bodies), a new desktop end-to-end file with 5 tests (picking, replacing, removing and refusing a file, a missing or broken file, a value of a later version, and the layout at 375 and 320 CSS px) and a new Android end-to-end file with 4 tests (the Theme list on a phone, a font file in app storage and in a shared folder, also after a restart, and the system's file picker). The tests in `settings_compat.test.ts` also pass against the code of 1.0.0, 1.1.0 and 1.2.0, and each commit of the Theme list, the high-contrast themes and the font file passes its own tests. The changes to the end-to-end harness add no tests.

Tests for 1.3.0: 809 Rust tests (801 in `cairn-core`, `cairn-sync` and `cairn-server`, also run with `CAIRN_FUZZ_SEEDS=200`, plus 8 unit tests in the app crate), 486 Vitest tests, 536 desktop end-to-end tests and 107 Android end-to-end tests pass. The others are 36 Rust tests marked ignored (18 reproductions, 7 tests that pass and show behavior by design, and 11 slow measurements and probes), the reproductions marked todo (12 desktop tests) or expected to fail (1 Vitest test), 12 desktop tests that run only when an environment variable turns them on (performance, memory, a 50 MB note and slow plugin checks), and 3 desktop tests that skip when the title bar's close button cannot be reached through the accessibility bus. On Windows, CI runs 680 of the Rust tests (673 in the three crates and 7 in the app crate) and the 486 Vitest tests, and 50 Rust tests are marked ignored there.

## 9. Known limits

These hold in version 1.3.0. Most FINDING numbers also name the tests that reproduce or check that case; no test names FINDING-031, FINDING-033, FINDING-127, FINDING-128, FINDING-151, FINDING-163, FINDING-165 or FINDING-176. The known gaps at the end of v2 and v3 in section 8 also still hold, except that Live Preview embeds on desktop refresh when the embedded file changes (FINDING-092), a client stops when the server has fewer changes than it has seen (FINDING-058), release APKs are signed with the project's release key (see the release notes), and Windows is no longer wholly untested: CI builds and tests Cairn there, and releases have a Windows installer, but nobody has tried the app by hand on Windows (see Windows, below). macOS is still untested. Since 1.3.0, a matching bracket, special characters, the drop cursor and the graph's hover box take the theme's colors, a focused text field in Settings shows the accent border, and links to missing notes in the outgoing links panel are dashed (section 8).

### Files and links

- A rename made in Cairn refuses a name that differs only in case from another file or folder in the same folder, as creating does. Files made or renamed outside Cairn, and new or renamed files that sync brings from another device, skip this check, so on Linux and in Android app storage a notebook can hold two notes whose names differ only in case. A macOS or Windows device then gives one of them a conflict copy name, and an Android shared folder keeps one and lists the other under "Files not synced" (FINDING-053).
- By design, names with a backslash do not sync. The app lists them under "Files not synced" with "The name contains a backslash. Rename it to sync it." A synced note renamed outside Cairn to such a name counts as deleted, so the other devices move it to their trash (FINDING-011).
- When the plain file of a Unicode twin goes, the twin takes the plain name at the next full rescan, and the other devices get it under that name through sync (FINDING-011).
- A file reached under two names through symlinks (a link to a folder of the notebook, two links to one folder, or a symlink to a note) syncs under one name. Version 1.1.0 and older synced it under both, so other devices may have a second copy. That copy is no longer updated, with two exceptions: deleting the note on the device with the link deletes the copy too, unless another device changed the copy meanwhile, and when another device deletes the note's own copy and keeps the second one, the second one syncs in its place. An edit or rename of the second copy waits on the device with the link and is listed under "Files not synced". A device that still runs 1.1.0 or older with the link moves the real note to the trash when another device deletes either copy (for a symlink to a note, the note's own copy), and its next sync deletes the other copy on every device, so every such device must be updated first. Two devices whose links differ for the same files can sync different names of one file; each then holds and lists the other's changes. While a folder that holds the link cannot be read, a delete of the note's own copy from another device waits, and a new file with the content of a note in that folder is not uploaded; both are listed. After a restart in that state, an edit made through the other name meanwhile can sync as a second copy. After the sync state is rebuilt, an older copy that differs from the note waits, listed, and so does a delete of the note from another device (FINDING-224).
- A remote move of a synced symlink to a file into another folder waits, listed, when the link's target is a relative path, because the move can break the link. Cairn does not check whether it would. This holds for any such link, not only one that gives a note a second name (FINDING-224).
- A rename or move made in Cairn refuses a Windows device name (CON, PRN, AUX, NUL, COM0 to COM9 and LPT0 to LPT9, also with ¹, ² or ³, CONIN$ and CONOUT$, with any extension) on every platform. Renames that sync applies still take such a name, except on Windows, where sync lists the file under "Files not synced". So a note such as con.md that was made outside Cairn or came from another device cannot be moved to another folder in Cairn under that name; rename it first.
- Files with `#`, `|`, `[` or `]` in their names are left out of the `[[` autocomplete, because no wikilink can name them. Offering them there as Markdown links is deferred. Of two files whose names differ only in case, only the one that wikilinks lead to is offered (FINDING-184).
- Cairn does not save settings, snippets or a font file through a `.cairn`, `.cairn/snippets` or `.cairn/fonts` folder that is a link out of the notebook, and does not move a font file to the trash through one: it shows an error and writes nothing outside the notebook. Reads still follow such links, `.cairn/plugins` and `.cairn/fonts` included, so plugin code and the font file can come from outside the notebook, though a plugin still needs the user's approval (FINDING-013). Cairn refuses the settings, a snippet or a plugin in `.cairn/` that is larger than 16 MB, and a plugin file that large can keep Settings, then Plugins, from listing any plugin.
- A hard-linked note is written in place after a temp copy. A crash during that write can leave the note half written under all its names, with the full text in a `.cairn-tmp-*` file next to it. If the write fails, Cairn renames the copy over the note, so the link splits and the other names can be left half written (FINDING-013). On Windows, a save always splits the link (see Windows, below).
- Images in dot-folders do not show, in a note or in an image tab, and the tree, the quick switcher and search do not list them (FINDING-022).
- Image tabs are not reopened when Cairn starts again. Versions 1.0.0 and 1.1.0 open every tab of the saved session that is not the graph as a note, so the session keeps the format they read, and they still restore the other tabs from it.
- Search lists images by their path only, and a query with a tag or a quoted phrase lists none.
- An external edit that keeps the size and mtime (`rsync -t`, `cp -p`, coarse FAT or SMB timestamps) is not picked up until the file changes again; a save from an open tab still compares the content (FINDING-046).
- An external save that lands between Cairn's last check and its rename can be overwritten. In testing, none of 500 saves at a 50 ms cadence were overwritten, and about 2% were in a back-to-back stress test. This window is accepted as a known limit (FINDING-047).
- On desktop, a save cut off by a crash leaves a `.cairn-tmp-*` file next to the note. Cairn does not list it, and on Linux it is hidden, but on Windows a leading dot does not hide a file, so File Explorer shows it (FINDING-177).

### Sync

- The server stores each device's name in plaintext. Encrypting it is later work (FINDING-152).
- A remote change this device cannot apply (a name its file system refuses, a folder it cannot read or write) stays pending and is listed under "Files not synced", and that file's local changes wait with it. On Windows this holds for a name with `<`, `>`, `:`, `"`, `|`, `?` or `*`, a name that ends in a dot or a space, and a device name such as con.md, which other systems allow; such a file is listed with "Windows does not allow the name". A file on one device and a folder of the same name on another wait until one side renames (FINDING-017, FINDING-057).
- Files over about 150 MB do not sync. The app lists them under "Files not synced" and does not read them for upload. A larger record already on a server stops every pull. A pull holds about one 64 MB batch of changes plus one page of up to 200 MB in memory, and each sync reads every pending change into memory when it starts. When the sync state is rebuilt, which includes the first sync of a notebook folder that already has files, the changes beyond the first batch are downloaded twice in that sync (FINDING-016, FINDING-019).
- Deleting every file in the notebook on purpose syncs only once the notebook has a file again. Every file counts, not only notes, but files in hidden folders do not, so when images or other files stay, deleting every note syncs at once (FINDING-006).
- A reset server is noticed only while it has fewer changes than the device has seen (FINDING-058).
- After the sync state is rebuilt (lost or damaged, or sync turned off and on), notes deleted elsewhere come back, because sync makes no automatic deletes (FINDING-054).
- When sync writes a remote change or moves a file to the trash, a file that another program writes in the few milliseconds after Cairn's last check is not protected (FINDING-015).
- On Windows and in Android shared folders, sync misses an edit made outside Cairn that keeps a note's size and puts back a modification time more than 3 s old (`cp -p`, `rsync -t`, a backup restore). It also misses another file of the same size and modification time moved over a note: the other devices keep the note's old text, and if that file had already synced elsewhere in the notebook, they move it to their trash. The new text is uploaded when the note changes again, and if another device edits the note first, the two versions are merged (FINDING-055).
- Downloaded files get the time of the download as their modification time, so the quick switcher's recent order on other devices follows the sync. Applying the original time needs a way to set it through `VaultFs`, Android shared folders included (FINDING-150).
- An empty note renamed outside Cairn syncs as a delete and a create, and so do identical notes renamed outside Cairn before one sync when two or more of them keep neither their file names nor their folders. Other devices move the old copies to their trash, and nothing is lost (FINDING-136).
- A note renamed outside Cairn and edited before the next sync syncs as a delete and a new note, because sync does not match notes by similarity. Its version history starts again, other devices move the old note to their trash, and an edit made elsewhere in the meantime comes back under the old name. A rename made in Cairn while sync is set up keeps the history (FINDING-062).
- Restoring a version needs a connection (FINDING-024). The version history of a note renamed and edited in Cairn shows only after the next sync, about 4 s later (FINDING-165).
- A note deleted on one device inside a folder that another device renamed comes back in the renamed folder, whichever device syncs first. A rename that sync makes itself (to a conflict copy name) also beats a delete once it is uploaded, so a note deleted on one device can come back under a conflict copy name (FINDING-148).
- When the kept one of two identical copies is both renamed and edited before a device that holds the deleted copy syncs, or is edited or renamed in the moment between another device's check and its delete, that device moves its own copy to its trash. The kept note stays, so nothing is lost (FINDING-063).
- Sync finds names that differ only in case with Unicode lowercase, so names that a file system folds another way (such as `ſ` and `s`) are not caught. In a Linux folder with case folding turned on, a case-only rename from another device is pushed back to the old name. Nothing is lost (FINDING-004).
- Sync setup asks before it creates a notebook the server does not have. Notebook names on the server are case-sensitive, so "Notes" and "notes" are two notebooks (FINDING-083, FINDING-164). The server cannot delete a notebook, so a notebook made by a setup that was then cancelled stays there (FINDING-163).
- A reverse proxy must not reuse idle connections to the server for more than 10 s; the bundled Caddyfile uses 5 s. A client that keeps opening new half-sent connections can still use up the server's file descriptors. Without a valid token, the server closes each one after at most 10 s. With a valid token, a request body that stops arriving is cut off after 30 s, but one that keeps sending a little at a time is not (FINDING-076).

### Windows

- Releases have a Windows installer, built in CI from the release's commit. It is not signed, so Windows SmartScreen warns before it runs, and on a PC with Smart App Control turned on, Windows can block the installer and Cairn outright. CI runs the Rust tests of the three crates and of the app crate, the frontend build, Vitest and clippy on Windows. The window and the end-to-end tests do not run there, and nobody has tried the app by hand on Windows.
- After a note is renamed only in case (note.md to Note.md), in Cairn or in File Explorer, the file tree can list it under both spellings. The old spelling goes when Cairn next reads the whole notebook: at the next sync, or when the notebook is opened again.
- When sync puts a note into a folder whose name differs only in case from a folder already there (Notes from another device, notes on this computer), the file tree can list that folder twice, once under each spelling.
- Cairn lists and syncs files that Windows marks as hidden or system files, such as desktop.ini and Thumbs.db, like any other file. Only a dot at the start of a name hides a file.
- Saving a note that has hard links replaces it with a new file, so its other names keep the old text, because the standard library cannot read a file's link count on Windows.
- A save from an open tab checks the note again just before it replaces it, but on Windows that check compares only the size and the modification time. Another program's save in that moment that keeps both (`cp -p`, `rsync -t`, a backup restore) is written over.
- A note that another program keeps open without allowing it to be replaced cannot be saved until that program closes it. Cairn then says it has no permission to access the note. A change that sync brings for it waits until then and is listed under "Files not synced", and, as in File Explorer, the note cannot be renamed, moved or deleted while it is open.
- A file whose name no Windows file can have, made on another system on a drive that Windows also reads, is left out of the notebook and not listed under "Files not synced".
- A file or folder whose name ends in a dot or a space, or is a device name such as CON, can still be in the notebook if another program made it. Deleting it, or anything inside a folder with such a name, moves it to the notebook's `.trash` folder, not to the Recycle Bin, because the Recycle Bin would take another file for it. Cairn's file tree does not show `.trash`, but File Explorer does.
- Found by reading the code, and not tried on Windows:
  - F5 and Ctrl+R can reload Cairn's window, as they reload a page in a web browser.
  - A character typed with AltGr, such as @ or € on many keyboard layouts, can count as Ctrl+Alt with that key. It can then run a command whose hotkey is Ctrl+Alt with that key, and the hotkey recorder can record it that way. No default hotkey uses Ctrl+Alt.
  - A notebook on a network share (`\\server\share`), also one opened through a mapped drive letter such as `Z:`, shows and keeps its folder with a `\\?\UNC\` prefix, and Cairn passes paths in that form when it opens a file in its default app or shows it in File Explorer. A path longer than about 260 characters keeps its `\\?\` prefix in the same way.
  - Sync setup suggests "windows" as the device name, not the computer's name. Conflict copies carry the device name, so give each Windows computer its own name in sync setup.
  - Each notebook's sync settings, the server token, the notebook key, the sync state, the copies of synced notes kept for merging, the plugin approvals and the list of recent notebooks are in the roaming AppData folder. Where roaming profiles are in use, Windows copies that folder to the other computers the user signs in to. A plugin turned on for a notebook there is then approved for a notebook at the same path on those computers: it runs there without being turned on again if that notebook lists it and the plugin file is the same.
  - When many files change outside Cairn at once, the file watcher can miss some of the changes without noticing. After an error, such as a network share that drops, it can stop watching the notebook until the notebook is opened again. Cairn finds the missed changes at the next sync, or when the notebook is opened again.
  - Signing out, shutting down or restarting Windows ends Cairn at once, without the save that closing the window makes. Edits made since the last autosave are lost, and so are edits that Cairn could not save, which it would ask about when the window closes.
  - Where the Recycle Bin cannot take a file or folder (a notebook on a network share or a USB stick, or a file larger than the bin allows), Windows can ask whether to delete it for good when Cairn deletes it, also when sync deletes it in the background. Yes deletes it for good; No moves it to the notebook's `.trash` folder.
- 129 Rust tests that run on Linux do not run on Windows, 1 of them in the app crate:
  - 72 need Unix mode bits (unreadable or read-only files and folders, kept file modes, the executable bit), symlinks (Windows needs Developer Mode or admin rights to make one), or inode numbers and link counts. 31 of them were Unix-only before the tests first ran on Windows, and 2 were written Unix-only since; the other 39 were limited to Unix for Windows.
  - 39 need two names in one folder that differ only in case, which a Windows folder cannot hold unless case sensitivity is turned on for it. 20 of them are built and marked ignored on Windows.
  - 9 need a name that Windows refuses: a backslash, a colon or a question mark.
  - 7 need gdb with the debug info of a Linux build, `/proc`, `ulimit` or the Unix limit on path length.
  - 2 check the change stamp, which `StdFs` does not have on Windows (FINDING-055). They are marked ignored there.
- Some tests check less on Windows:
  - The test of odd names made outside Cairn leaves out the characters Windows refuses.
  - Three tests leave out their steps with names that differ only in case: the random test of the index, the test of renames in the app, and the phone test of a note under another spelling.
  - While a folder is being deleted, Windows reports it as access denied, so the test of a folder replaced during a scan checks only the notebook's rescan, which skips such a folder for that scan.
  - The test of device names leaves out its last step, a rename by sync to such a name, which Windows cannot make.
  - The test that the server answers before the body arrives sends no body on Windows, where a TCP reset discards an answer that has not been read yet.

### macOS

- Cairn has not been built, tested or run on macOS, CI does not cover it, and releases have no macOS build.

### Android

- Shared folders refuse a name that differs only in case from an existing one, and names the storage cannot hold. A file synced from another device under such a name, or into a folder whose name differs only in case from one on the phone, is not stored on the phone and is listed under "Files not synced" (FINDING-031, FINDING-172).
- Of two notes whose names differ only in case, the phone stores one in the shared folder and lists the other. When another device renames a note to a name that differs only in case from another note on the phone, the phone holds the rename back and lists it. Edits made to that note on the phone stay on the phone until one of the two notes is renamed or deleted. If the phone already has a copy of another device's note in a folder whose name differs only in case from that device's folder, the copy syncs as a second note, because taking it for that note would mean renaming the folder on every device. Nothing is lost in any of these cases (FINDING-172).
- When another device moves some notes into a folder whose name differs only in case, the phone renames the whole folder, on every device (FINDING-034).
- Shared folders have no atomic replace: a kill during a save can leave the new text followed by the end of the old, never a shorter file, and a kill during a move under a new name can leave the note under an intermediate name (FINDING-026, FINDING-033).
- After a renderer crash, text typed within the autosave delay is lost (FINDING-176).
- Cairn cannot open files in other apps yet, images included. Images open in a tab inside Cairn, and a tap on another attachment shows a message. In Live Preview a tap on an embedded image edits the embed, so the image opens from the reading view. An image in a shared folder is read whole through the Storage Access Framework before it shows, so a very large photo there takes a while to open.

### Plugins

- Approvals are per device and per exact file content, so editing a plugin or moving the notebook turns it off until the user turns it on again.
- Plugins have no memory limit: Cairn sets none, and a web worker cannot be given one. A plugin that keeps allocating can end the web view's process; on Linux the window then goes blank (FINDING-161). This has not been tried on Android or on Windows.
- Cairn stops a plugin and turns it off when a command runs over 30 s, the plugin misses three pings or it floods the app with messages. The only way to stop a slow command is to turn the plugin off (FINDING-069, FINDING-153, FINDING-156, FINDING-162).
- A plugin has at most 200 commands and 16 API calls in flight, and at most 5 toasts show at once (FINDING-072, FINDING-157, FINDING-162).

### Core plugins (1.1.0)

- Their switches and options are in `.cairn/settings.json`, which Cairn sync does not copy, so with Cairn sync they are set up on each device.
- Day and month names in dates are always English, so that devices with different system languages give a daily note the same name.
- In a daily note's template, `{{date}}` and `{{time}}` use the date and time formats of the Templates plugin, whether it is on or not.
- There is no option to open today's note when a notebook opens: nothing is created by opening a notebook.
- Thai, Lao, Khmer and Myanmar word counts come from the web view's dictionary (ICU), which is not the same in WebKitGTK, in WebView2 on Windows and in Android's web view, so they can differ a little between devices.

### Themes (1.2.0)

- Cairn 1.0.0 and 1.1.0 show Limestone or Slate in a notebook set to Marble or Graphite. They keep the choice, so it comes back in 1.2.0 and later.
- On Android, the area behind the status and navigation bars follows the system's light or dark mode, not the theme chosen in Cairn. On the desktop (tested on Linux), the window's title bar does the same.
- In Limestone, Marble, Slate and Graphite, scrollbars, horizontal rules and graph edges are below 3:1 against their background, and the highlighted entry of the command palette and quick switcher, the selected version in version history and pressed panel buttons show only as a tint or a color. Since 1.3.0, the high-contrast themes keep scrollbars, horizontal rules and graph edges at 3:1 or more, and draw rings and bars of at least 3:1 for those states.
- A CSS snippet can change any color, so it can bring a theme below its contrast.

### High-contrast themes (1.3.0)

- Cairn 1.0.0 to 1.2.0 show Limestone or Slate in a notebook set to a high-contrast theme. They keep the choice, so it comes back in 1.3.0 and later.
- The high-contrast themes do not follow the system's high-contrast setting; they are picked in Settings. WebKitGTK reports that setting only for a GTK theme named HighContrast, and Android's web view does not report it. What a Windows contrast theme does to Cairn's colors has not been tried.

### Font file (1.3.0)

- A notebook has one font file. The web views draw bold and italic text from it by thickening and slanting the regular letters, also when the file is a variable font.
- Cairn sync does not copy `.cairn/`, settings included, so another device uses the Text font until the font file is picked there too. Copying the file alone is not enough: the device's own `textFont` setting names it. Cairn 1.2.0 and older use the Text font too.
- A font file picked under the name of the one in use (ignoring case) is saved over it; one of another name replaces it and the old one goes to the trash.
- The web views use only the first font of a collection, so ttc files are refused. Android's web view and WebView2 on Windows check fonts more strictly than WebKitGTK, so a file that loads on Linux can fail on a phone or on Windows; Cairn then uses the Text font and says why.

### Editor and app

- On Linux, a SIGTERM to the app saves open notes first, as closing the window does, and the app exits at most 3 s later. When every process of the app gets a signal at once (systemd does this at logout or shutdown, and so do Ctrl+C and a closed terminal), the web view goes too, and the edits made since the last autosave are lost. SIGKILL or a crash loses them as well. Autosave runs 600 ms after the last keystroke (FINDING-009).
- Unsaved edits are merged with a change on disk unless the changes touch the same or neighboring lines or either side rewrote more than about 10,000 lines (more than 20,000 lines added and removed); then the tab shows the banner that says the note changed on disk. Lines only added or only removed in one place, and a reordering of many lines, are not counted; a reordering can still make the merge slow, about 1 s for 20,000 reversed lines in a release build (FINDING-074, FINDING-151).
- If a note with unsaved edits is deleted outside Cairn and another note is then renamed onto its path in Cairn, two tabs show that path. "Save my version" in the first tab then shows the banner that says the note changed on disk, and that tab replaces the renamed note only if the user picks "Keep mine (overwrite)" (FINDING-010).
- Only `http:`, `https:` and `mailto:` links and a fixed list of file types (documents, images, audio, video, archives, and text files such as txt, csv, svg, json, yaml, ics and vcf) open with the system's default app. Web pages, XML and Office files with macros (html, htm, xml, docm, xlsm, pptm) do not. Windows opens svg in a browser, and old doc, xls and ppt files can hold macros (FINDING-066).
- The editor's highlighting ends frontmatter only at an exact `---` line (FINDING-052, FINDING-085).
- Editor keys such as Mod+Enter or Mod+D can be recorded as hotkeys, and then the editor does not get them (FINDING-096, FINDING-214).
- The zoom is not kept after a restart, the zoom keys cannot be rebound, and Ctrl+0 does not reset on AZERTY (on Linux; on Windows the zoom keys are the web view's own and were not tried). Android has no zoom keys (FINDING-125).
- Toasts have no history, and an error toast goes 7 s after the pointer and focus leave it (FINDING-218).
- Screen readers hear sync errors, recoveries and conflict counts, not each sync (FINDING-205).
- A nested folder moved to another parent right after an event on a file in it can be taken as a delete and a create (FINDING-129).
- Live Preview renders tables only as far as the editor has parsed (FINDING-043). When the editor runs out of time parsing a note, the next edit can hit a bug in @lezer/markdown that shows a code block's lines as plain text or a table and the text after the block as code, until the note is reopened. In testing, this took pasting a code block of about 90,000 characters or more a few thousand characters into a note, then one more edit (FINDING-225).

### Performance

- On 50,000 notes the graph is slow. On a debug build, a hover takes about 0.35 s (about 45 ms on 10,000 notes), and the current look stays (FINDING-042). Opening the graph takes 1.6 to 2.4 s on that build, with a longest frame of 0.7 to 1.1 s, and about 0.9 s on a release build, with one frame of about 730 ms (FINDING-127). The debug build uses about 1.6 GB after hovering, and closing the graph brings that down only to 1.3 to 1.4 GB (FINDING-128).
- Sync uploads one file per request, so the first sync of a large notebook takes time in line with its number of files (about 5.3 s for 5,000 notes in the sync tests). Sending files in batches needs a change to the sync protocol (FINDING-126).
