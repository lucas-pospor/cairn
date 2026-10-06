# Release notes

## 1.2.0 (2026-10-06)

### Images open in a tab

Images (PNG, JPEG, GIF, WebP, AVIF, SVG, BMP and ICO) now open in a tab inside Cairn, on the desktop and on Android. Before, the desktop handed them to the system's image viewer, and Android could not open them. They open from the file tree, the quick switcher (once you type; before that it still lists recent notes only), a link, search, and a click on an image embedded in a note in the reading view. In Live Preview, where a plain click puts the cursor on the embed to edit it, Ctrl+click or a middle click opens the image.

- The tab shrinks an image that is larger than the tab to fit it. Actual size, or a click on the image, shows it at full size, and the arrow keys scroll it.
- An SVG is shown only as an image, so scripts in it never run.
- The tab loads the image again when it changes on disk, follows it when it is renamed or moved, and closes when it is deleted.
- Search lists images whose name or folder matches the words, under the notes. A query with a tag or a quoted phrase lists none.
- The right sidebar lists the notes that link to the image.
- On the desktop, Open in default app, in the tab and in the file tree's menu, still hands an image to the system's viewer, and other attachments open there as before.

### Themes

Cairn now has four themes, each with a name. Limestone (the light theme so far) and Marble, a cool white theme with a blue accent, are light. Slate (the dark theme so far) and Graphite, a neutral gray with no blue tint, are dark. Settings, then Appearance, now has a Light theme and a Dark theme choice under Theme, and each theme shows a small swatch. Light and Dark use the chosen light or dark theme, and System switches between the two as the system switches between light and dark. The choice is saved in the vault's `.cairn/settings.json` under two new keys, `lightTheme` and `darkTheme`. On Android, the area behind the status and navigation bars still follows the system's light or dark mode, and so does the window's title bar on the desktop.

### Fixes

- Some colors were below the WCAG AA contrast ratio, 4.5:1 for text and 3:1 for the ring around a selected row under the pointer: code comments in the light theme, matched words in search results and code comments in the dark theme, and a few others in code blocks, on hovered rows, under matches of the selected word and in the sync error in Settings. They are now darker or lighter. The most visible change is in Slate: matched words in search results and other matches of the selected word have a darker highlight, which shows by its color more than its brightness.
- The editor's search (Ctrl+F) now highlights matches in the theme's own highlight color. Before, it always used a fixed yellow and orange, on which some text, such as code comments, was below 4.5:1. For this, the highlight is a little lighter in Limestone and a little darker in Slate.
- A custom accent color is now also adjusted to stay readable on hovered rows, in code and under matches of the selected word or of a search, so some accents come out a little darker in the light themes and a little lighter in the dark themes.
- The graph takes the new colors as soon as the theme, the accent or a snippet changes. Before, it kept the old ones until the next hover or reload.
- With no custom accent, the accent color picker shows the accent of the theme in use instead of always the light theme's.
- A file that the vault reaches under two names through symlinks (a link to a folder of the vault, two links to one folder, or a link to a note) now syncs under one name: the name it already syncs under, else the name with no link on the way, then the one in the fewest folders, then the first by character code. Before, both names synced, and deleting one copy on another device could move the real note to the trash and then delete the other copy on every device. Hard links sync as separate notes, as before. Compatibility, below, says what happens to the copies that older versions synced.
- When sync deletes the last note in a linked folder, it keeps the link instead of failing with "Not a directory". A folder that sync cannot remove after deleting or moving its last note no longer holds the change back.
- When another device moves a note to another folder, and on this device that note is a symlink whose target is a relative path, the move now waits and is listed under "Files not synced", because the move can break the link. To sync it, replace the link with a copy of the note.
- Alt+Enter on a Markdown link (`[text](path)`) finds the file relative to the note, as a click does.
- A plugin's `editor.getSelection` and `editor.replaceSelection` work only while a note is shown. Before, with the graph open, they read and changed the note last shown behind it.

### Compatibility

- The sync server and the sync protocol are unchanged, so a 1.0.0 or 1.1.0 server works with this version, and devices on 1.0.0 or 1.1.0 keep syncing with it.
- Copies that 1.1.0 and older synced under a file's second name stay on your other devices. Cairn no longer updates them or sends anything for them, with two exceptions: deleting the note on the device that has the link deletes its copy too (unless another device changed the copy meanwhile), and if another device deletes the note's own copy but keeps the second one, the second copy syncs in its place. If another device edits or renames such a copy, the device with the link lists it in Settings, then Sync, under "Files not synced": copy what you need into the note, then delete the copy on a device where it is a separate file. A device that has such a link and still runs 1.1.0 or older can still lose the note when another device deletes one of the copies, so update the devices that have the links first.
- Cairn 1.0.0 and 1.1.0 keep `lightTheme` and `darkTheme` when they save other settings. In a vault set to Marble or Graphite they show their own light or dark theme, and the choice comes back when the vault opens in this version again. The `theme` key still holds only system, light or dark, which is all that older versions read.
- Image tabs are not reopened when Cairn starts again. The saved session keeps the format of 1.1.0, so 1.0.0 and 1.1.0 still restore the other tabs from it.
- On Android, images open in a tab as on the desktop, but Cairn still cannot open files in other apps there, images included: the tab has no Open in default app, and a tap on another attachment shows a message, as before. A phone has no Ctrl+click or middle click, so tap an embedded image in the reading view to open it. An image in a shared folder is read whole before it shows, so a very large photo there takes a while to open.

## 1.1.0 (2026-10-06)

### Core plugins

Cairn now comes with optional built-in features, turned on and off under Settings, then Core plugins. The README describes each one.

- Templates (on by default) inserts a note from the template folder at the cursor, with `{{title}}`, `{{date}}` and `{{time}}` filled in.
- Daily notes (on by default) opens today's note, named by the date, and creates it when there is none, from a template if you set one.
- Unique note creator (off by default) creates a note named by the date and time.
- Random note (off by default) opens a note picked at random.

They never write over a note, and none of them does anything when a vault opens. They need no approval on each device, since they are part of the app. Their switches and options are saved in the vault's `.cairn/settings.json` under a new `corePlugins` key. Cairn 1.0.0 keeps that key when it saves other settings, so a vault can move between versions without losing them.

### Word count

The status bar now shows characters as well as words, and counts the selection while there is one. In Chinese and Japanese text, each Han, Hiragana or Katakana character counts as a word, and Thai, Lao, Khmer and Myanmar text is split into words with the system's dictionary, so these counts are higher than in 1.0.0, which counted a run of such text as one word. Thai counts can differ a little between the desktop and Android, whose dictionaries differ.

## 1.0.0 (2026-10-05)

This is the first public release of Cairn, a local-first Markdown notes app. A vault is an ordinary folder of `.md` files, and Cairn reads and writes them directly. The [README](../README.md) covers building from source, the sync server and plugins, and section 9 of [PLAN.md](PLAN.md#9-known-limits) lists every known limit. This release was tested on Arch Linux (GNOME, Wayland, WebKitGTK 2.52) and on an Android 15 emulator.

### Downloads

The GitHub release has these files, with their SHA-256 sums in `SHA256SUMS`:

- Linux (x86_64): `Cairn_1.0.0_amd64.deb`, `Cairn-1.0.0-1.x86_64.rpm` and `Cairn_1.0.0_amd64.AppImage`. The .deb and .rpm packages use the system's WebKitGTK 4.1 and GTK 3.
- Android 7.0 or later: `cairn-1.0.0-universal.apk`, for arm64, armv7, x86 and x86_64 devices. Its signing certificate has the SHA-256 fingerprint `1D:10:FE:DC:B9:A2:55:63:DA:A0:59:AD:79:30:64:15:3D:A8:7F:92:C4:0C:49:A8:98:F2:00:AF:09:B9:82:8C`, and later versions are signed with the same key.
- Sync server: `cairn-server-1.0.0-docker-image.tar.gz`, the Docker image `cairn-server:1.0.0`. Load it with `docker load -i cairn-server-1.0.0-docker-image.tar.gz`.

There are no macOS or Windows builds. You can build them from source as the README describes, but they have not been tested.

### What it does

- Notes: Live Preview editing (with source mode, reading view and a split view), `[[wikilinks]]` with autocomplete and Obsidian's resolution rules, embeds, pasted attachments, and backlinks, outline, properties and tags panels.
- Finding things: a quick switcher, a command palette, full-text search with `tag:` and `path:` filters, and a graph view.
- Your setup: light and dark themes, an accent color, fonts, CSS snippets and customizable hotkeys.
- Sync: end-to-end encrypted through a small server you run yourself. Edits that collide become conflict copies instead of being lost, version history restores older versions, and a file that cannot sync is listed under "Files not synced" while the rest syncs.
- Android: the same app with a touch layout and a formatting toolbar. A vault lives in the app's own storage or in a shared folder that other apps can open too.
- Plugins: JavaScript files in `.cairn/plugins/`, each in its own sandbox. A plugin runs only after you turn it on under Settings, then Plugins, on each device, and it can reach only the vault's Markdown notes.
- Accessibility: the file tree, tabs, menus and graph work from the keyboard, text meets WCAG AA contrast, and screen readers announce sync errors and search result counts.

### Known limits

The main ones (section 9 of [PLAN.md](PLAN.md#9-known-limits) has the full list):

- On Linux, systemd stops all of Cairn's processes at once at logout or shutdown, so the last 0.6 seconds of typing can be lost. Pressing Ctrl+C in the terminal that started Cairn, or closing that terminal, does the same.
- After a reconnect or a lost sync state, Cairn deletes nothing, so a note deleted on another device comes back. That sync, like the first sync of a folder that already has files, downloads everything past the first 64 MB of the server's changes twice. On a phone, that doubles the data the sync uses.
- When symlinks that do not loop make a folder reachable under two names, the folder syncs under both names. If another device deletes one copy, the real notes go to the trash on every device. A fix is planned.
- Files and folders with a backslash in their name do not sync. If a synced note is renamed outside Cairn to such a name, the other devices move it to their trash.
- The server stores each device's name in plaintext. Encrypting the names is left for later work.
- Sync does not apply modification times, so a downloaded file has the time of the sync, and the quick switcher's recent order differs between devices.
- In an Android shared folder, the phone stores one of two notes whose names differ only in case and lists the other. When another device renames a note to a name that differs only in case from another note on the phone, the phone holds the rename back, and edits made to that note on the phone do not sync until one of the two notes is renamed or deleted. Nothing is lost.
- Uploads go one file per request. A batch upload would need a change to the sync protocol.
- Plugins have no memory limit. A plugin that keeps allocating memory can make the window go blank.
- The graph is slow on 50,000 notes. On a debug build it takes 1.6 to 2.4 seconds to open, a hover takes about 0.35 s, and the app uses about 1.6 GB. Closing the graph frees only part of that memory.
- Error toasts disappear 7 seconds after the pointer and focus leave them, and there is no message history.
- After a very large code block (about 90,000 characters or more in testing) is pasted into a note and the note is edited again, a bug in the @lezer/markdown parser can make Live Preview show the code block's lines as plain text or a table and the text after it as code. This stays until you reopen the note. The note's text does not change.
- On Android, Cairn cannot open attachments in other apps yet, and there are no zoom keys.
