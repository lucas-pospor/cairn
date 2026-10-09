# Release notes

## 1.4.1 (2026-10-09)

### Windows installer

- Shows Cairn's icon on the Windows installer and uninstaller, and the Cairn logo on the installer's pages.

### Fixes

- Saves open notes when Windows signs out, shuts down or restarts, and when an installer closes Cairn. While a note holds edits that cannot be saved, Cairn keeps Windows from signing out, shutting down or restarting and names the note.
- Keeps saving on Windows after Alt, F10, or Alt+Space and Esc, which left saves waiting until the next key or click.
- Redoes with Ctrl+Shift+Z in the editor on Windows.
- Refuses to open a file in another app when Windows apps cannot take its path, with an error instead of an app that fails.
- Keeps the device name and the notebook name empty in Settings, then Sync, once you delete them.
- Leaves a right-click in the rename box to the text box, which no longer renames the file to the half-typed name.
- Shows a repeated message once while it is on screen, instead of stacking copies of it.
- Lets the installer go on when an older Cairn's install folder is gone.

### Compatibility

- Works with servers and devices on 1.0.0 to 1.4.0: the sync protocol, the server's API and the formats on disk are unchanged.
- Installs over 1.4.0 and older as an update and keeps the app's data, its notebooks and its sync setup. The Windows installer is still not signed.

## 1.4.0 (2026-10-08)

### Manual and website

The user documentation moved out of the README into a manual in `docs/manual`, which the website at https://lucas-pospor.github.io/cairn/ also shows, together with a download page for the latest release.

### Fixes

These fixes come from two tests of Cairn by hand on Windows 11.

- On Windows, on a 1920 by 1080 screen at 125% scaling, the window opened with its bottom, status bar included, under the taskbar. The window now opens centered on the primary monitor, and smaller when its usual size would not fit in the area that the taskbar leaves free. This also holds on Linux with X11. On Wayland the window still opens where the system puts it.
- On Windows, F5, Ctrl+R, Shift+F5, Ctrl+Shift+R and Ctrl+F5 reloaded the window, as in a web browser, and so did Refresh in the right-click menu. A reload drops, without a question, the edits that Cairn could not save, such as those of a read-only note or of a note in conflict with the disk. The web view also acted on its other browser keys wherever Cairn did not use them: Ctrl+P with Settings open opened Print, Ctrl+F outside the editor opened its find bar, and F7 asked about caret browsing. These keys now reach only Cairn, so none of this happens, while Cairn's hotkeys, the editor's keys and the zoom keys work as before. With a WebView2 Runtime older than 120.0.2210, only the reload keys are taken from the web view, and Cairn does not get them either, so a hotkey set to F5 or Ctrl+R does not run there. The right-click menu keeps only Emoji, its editing items, the spelling suggestions, Copy link and Copy image. Back, Refresh, Save as, Print and More tools are gone. Outside text fields, a selection, a link or an image, the web view shows no menu. In the reading view, Ctrl+F now does nothing: press Ctrl+E to edit the note and use Ctrl+F there, or search the notebook with Ctrl+Shift+F.
- Where the window can still be reloaded, such as with Reload in the right-click menu on Linux, the web view now asks first while a note holds edits that could not be saved or are in conflict with the disk, or a change of the settings could not be written. Edits that wait for autosave do not count: they are saved as before.
- Two Windows or macOS devices could spell one folder differently, such as Drafts on one and DRAFTS on the other, also after a case-only rename of the folder in File Explorer. Sync then moved every note in that folder back and forth, for ever. Each sync stored another full copy of each of those notes on the server. The file tree could also list the folder twice. Now each device keeps its own spelling, and a note moves on the server only when it really moves. On an Android phone with the notebook in shared storage, a new note from another device in a folder that the phone spells otherwise is not stored on the phone. It is listed under "Files not synced" in Settings, then Sync. Before, the endless moves renamed the phone's folder to the other device's spelling, so the phone did store those notes. Update every Windows and macOS device that syncs such a notebook.
- On Windows, deleting a note, a folder or the font file in a notebook on a network share or a mapped drive deleted it for good, without a question, and so did a delete that sync applied there. Windows deletes for good what the Recycle Bin cannot take, and it told Cairn that the delete had worked. Cairn now moves such an item to the notebook's `.trash` folder. This holds on a share or a mapped drive, on a USB stick or another drive without a Recycle Bin, and on a drive set to remove files at once ("Don't move files to the Recycle Bin"). It also holds for an item larger than the Recycle Bin of its drive takes. Cairn reads that size from the Recycle Bin's settings, or works out the size Windows gives the bin by default, and takes that size a little smaller, so an item close to it goes to `.trash` too. So does an item when Cairn cannot read its size or the Recycle Bin's. If Windows still deletes an item for good, the delete fails with an error that says so, instead of looking as if it had worked. An item at a path longer than about 260 characters also goes to `.trash` now, where before it went to the Recycle Bin. File Explorer shows `.trash`; Cairn's file tree does not. A note behind a folder link to a share, or to another drive whose Recycle Bin cannot take it, cannot be deleted in Cairn: the delete fails with an error and the note stays.
- On Windows, Reveal in file manager did nothing, and showed no error, for a notebook on a network share or a mapped drive, or at a path longer than about 260 characters. It now works on shares and mapped drives. When File Explorer cannot show a file, because its path is too long or has a name that File Explorer cannot handle, Cairn says so. Errors from Reveal in file manager and Open in default app are now plain sentences, also on Linux, and the command palette no longer offers Reveal current note in file manager on Android, where the file tree's menu already left it out.
- A delete that failed, such as a delete on Windows of a note that another program held open, left an empty `.trash` folder in the notebook, which File Explorer shows. A failed delete now leaves nothing behind.
- Sync setup suggested "windows" as the device name on every Windows computer, so conflict copies from two Windows computers carried the same name. It now suggests the computer's name. A computer that is already set up keeps the name it has. On Linux, an empty `HOSTNAME` variable no longer hides the host name.

### Compatibility

- The sync protocol and the sync server's API are unchanged, so servers and devices on 1.0.0 to 1.3.1 keep working with this version.
- The sync state of a notebook gains an optional field: the server's spelling of a file whose folder this device spells otherwise. Older versions ignore it, and a sync state they wrote loads with it empty.
- Nothing else changes on disk: notes, settings, the session and the plugin approvals keep their formats.
- The desktop installers and the APK install over 1.3.1 and older as an update and keep the app's data, its notebooks and its sync setup. The Windows installer is still not signed.
- A device still on 1.3.1 or older applies another device's spelling of such a folder and sends the notes back under its own spelling, as before. Two devices caught in that loop settle once both are updated: each sends its own spelling of a note once more, and the other takes it for the server's spelling.

## 1.3.1 (2026-10-07)

### Fixes

- On Android 17, sync to a server on the local network, such as one at home with an address like 192.168.x.x, failed after about 30 seconds with "cannot reach the server: it did not answer in time". Android 17 lets apps reach the local network only with the Nearby devices permission, which Cairn did not have. Cairn now asks for it when you start a sync yourself (Connect and sync, Sync now or a tap on the sync status) or open a note's version history, and the server is on the local network. If you refuse, sync stops at once with a message that says how to allow it later in Android settings (Apps, then Cairn, then Permissions, then Nearby devices). Syncs that run by themselves never show the prompt; while the permission is missing, they stop with a message that says how to allow it. Older Android versions do not have this permission, and nothing changes there.

### Compatibility

- The sync protocol and the sync server's API are unchanged, so servers and devices on 1.0.0 to 1.3.0 keep working with this version. Nothing changes on disk.
- Apart from the version number, the desktop apps and the sync server are the same as in 1.3.0.
- The APK installs over 1.3.0 and older as an update and keeps the app's data, its notebooks and its sync setup. On Android 17 it asks for one new permission, Nearby devices, and only when a sync you start, or a note's version history, goes to a server on the local network.
- Cairn treats a server as on the local network when its address is private (10.x.x.x, 172.16.x.x to 172.31.x.x, 192.168.x.x), in 100.64.x.x to 100.127.x.x (shared address space, which some VPNs use), link-local or an IPv6 unique local address, or its name ends in `.local`. It does not ask when such a server answers without the permission, as one reached through a VPN does: Android does not block a VPN. A server on your network with a public IPv6 address is not recognized as local, although Android counts it as local: without the permission, its sync still times out, and the message then also says to allow Nearby devices.

## 1.3.0 (2026-10-07)

### Notebooks

Cairn now calls the folder of notes a notebook. Before, it said vault. Only the word changes: notebooks open, sync and keep their settings as before. The `CAIRN_VAULT` variable keeps its name, and so does the sync server's API, so devices and servers on older versions keep working with this one. The top of a notebook is called the notebook folder, for example in the list of folders that Move to offers and in the Folder option of Daily notes and Unique note creator. The sync server still answers "no such vault" and "vault exists", which Cairn now shows as "no such notebook" and "a notebook with this name already exists", and its log now says "created notebook" where it said "created vault".

### One Theme list

Settings, then Appearance, now has one Theme list in place of the Theme choice and the Light theme and Dark theme rows. It holds System, then every theme by name, light and dark, with a swatch of the theme in use beside it. Picking a theme uses it always. With System, two small lists under it, Light and Dark, pick the theme to use while the system is light and while it is dark. Picking a light theme in the Theme list also makes it the Light one, and a dark theme the Dark one, as the rows of 1.2.0 did.

### High-contrast themes

Two new themes, High contrast light (black on white) and High contrast dark (white on black), keep all text at 7:1 or more against everything it is drawn on, which is WCAG AAA: muted text, Markdown marks, code colors, links and error text included. Only disabled buttons, which WCAG does not count, and a file tree row while it is dragged are dimmed below that. Borders and the scrollbar keep 3:1. Selected rows in the command palette, the quick switcher, version history, menus and autocomplete, the open note in the file tree, the Settings section shown, pressed panel buttons and focused controls get a ring or a bar in the accent color, so none of them shows by a tint alone. A custom accent color is darkened or lightened to 7:1 in these themes. They are not picked by the system's high-contrast setting; choose them under Settings, then Appearance. What a Windows contrast theme does to Cairn's colors has not been tried.

### Your own font

Under Settings, then Appearance, then Font file, you can pick a font file for note text: woff2, woff, ttf or otf, up to 20 MB. Cairn saves it in the notebook's `.cairn/fonts/` and names it in `.cairn/settings.json` under a new key, `textFont`, so it moves with the notebook folder. The Text font choice stays and is used wherever the font file is not: on another device where it has not been picked (Cairn sync copies neither the file nor the settings), if the file is missing or broken, and in older versions. Picking a file with another name, or Remove, moves the old one to the trash. A file with the same name, ignoring case, is saved over it and keeps the old name. A file that is not one of these fonts, a font collection (ttc), or a file over 20 MB is refused with a message, and nothing is saved.

### Windows installer

Cairn now has an installer for Windows 10 and 11 on x86_64, `Cairn_1.3.0_x64-setup.exe` on the release page. It is built on GitHub's Windows runners from the release's commit, where most of the Rust tests and the frontend unit tests also run. The window and the end-to-end tests do not run there, and nobody has tried the app by hand on Windows yet. Section 9 of PLAN.md lists the tests that do not run on Windows and what is known not to work there.

- The installer is not signed, so Windows SmartScreen warns before it runs (Compatibility, below, says what to do).
- It installs Cairn for your user account only, without administrator rights, by default in `%LOCALAPPDATA%\Cairn` (you can choose another folder), with a shortcut in the Start menu and, if you leave its box ticked, one on the desktop. Windows lists Cairn with the installed apps in Settings, where you can uninstall it.
- Cairn needs the WebView2 Runtime, which Windows 11 and an up-to-date Windows 10 already have. If it is missing, the installer downloads it from Microsoft.
- Cairn keeps its own files (the recent notebooks, plugin approvals and sync state) in `%APPDATA%\app.cairn.notes`, and the data of its window in `%LOCALAPPDATA%\app.cairn.notes`. Uninstalling leaves both unless you tick "Delete the application data". Your notebooks are never touched.

### Fixes

- In every theme, a matching bracket, a bracket with no match, the mark the editor shows in place of an invisible or control character, the cursor shown while dragging text into a note, and the box behind a hovered label in the graph now take the theme's colors. Before, they kept CodeMirror's and the graph's own colors, on which some text was below 4.5:1 and the dragging cursor was black on the dark themes. For the outline of a matching bracket, Slate's input borders are a little lighter.
- Links to missing notes in the outgoing links panel are now dashed, as in the editor, so they do not differ from other links by color alone.
- A focused text field in Settings, such as the attachment folder, now shows the accent border as other text fields do, not only a faint tint.
- On the desktop, the card of an embedded file that Cairn does not open in another app (a program, a script, a file of an unknown type or with no extension, a link to such a file, or a text file marked as executable) said that the file opens in another app. The card now says that Cairn does not open it, as a click on its name already did.
- On Android, the card of an embedded file that does not show as text, such as a PDF, said that the file opens in another app. Cairn cannot do that on Android yet, and the card now says so, as a tap on its name already did.
- On Android, pasting or dropping a file into a note, such as an image, did not save it. Cairn showed "Could not save" with the error "Invalid path: expected raw bytes". The file is now saved in the attachment folder and linked in the note, as on the desktop.
- On Android, in a folder opened from storage, Settings listed no CSS snippets and no plugins, and snippets and plugins turned on in `.cairn/settings.json` did not load. A snippet made in Settings was saved but did not show. Settings now lists the files in `.cairn/snippets/` and `.cairn/plugins/` there too. As everywhere, a plugin runs only once you turn it on under Settings, then Plugins, on that device.
- Cairn now refuses the settings, a snippet or a plugin in `.cairn/` that is larger than 16 MB. Except in an Android folder opened from storage, it reads no more than that, and it refuses a link in `.cairn/` to a device or a pipe, which it used to read without end.
- Cairn no longer makes a note, folder or file in the notebook, such as a pasted or dropped attachment, with the name of a Windows device such as CON, NUL, COM1 or LPT1 (also with an extension), and no longer renames or moves one in Cairn to such a name. Many Windows programs, File Explorer among them, cannot open, rename or delete such a file. Cairn refuses these names as it already refused the characters Windows does not allow. On Linux, macOS and Android, sync still writes such a name that comes from another device, and Cairn still saves a snippet or a font file in `.cairn/` under such a name. On Windows, sync lists such a file under "Files not synced".
- On a file system that ignores case, as on macOS and Windows, the error for a new note or folder whose name differs only in case from another entry now names that entry, as it does on Linux.
- On Windows, a name in the notebook that starts with a drive letter and a colon, such as "D: plan.md", from another device could make Cairn read, write, rename or delete files outside the notebook folder, and a colon later in a name could reach a hidden stream of another file. Cairn on Windows now refuses every name that no Windows file can have, lists such a file from another device under "Files not synced", and makes no new file or folder whose name ends in a dot or a space or is a device name. Such a file or folder can still come from another program; deleting it, or something inside a folder with such a name, now moves it to the notebook's `.trash` folder, because Windows could move another file to the Recycle Bin in its place: deleting "Draft." could move "Draft". The notebook folder no longer shows with a `\\?\` prefix, and Open in default app and Reveal in file manager no longer hand that prefix to other programs, which many of them refuse. A notebook on a network share keeps it, and so can a very long path. A rename made in Cairn could also miss the next sync's list of renames, because Windows could not lock the file that records them; that is fixed too.

### Compatibility

- The sync protocol and the sync server's API are unchanged, so a 1.0.0, 1.1.0 or 1.2.0 server works with this version, and devices on those versions keep syncing with it. Only one line of the server's log changed.
- The new word renames nothing on disk: the notebook folder, its files and the names of the settings keys stay as they were. The `CAIRN_VAULT` variable keeps its name too.
- The high-contrast themes are stored in `lightTheme` and `darkTheme` like the others. Cairn 1.2.0 keeps them when it saves other settings and shows Limestone or Slate in their place; Cairn 1.0.0 and 1.1.0 do the same, as they do for Marble and Graphite. The `theme` key still holds only system, light or dark.
- Cairn 1.2.0 and older keep `textFont` when they save other settings and use the Text font, which is still saved under `fontFamily` as before.
- Cairn sync does not copy `.cairn/`, so the font file, like snippets and the settings, does not reach your other devices through it. Another device uses the Text font until the file is picked there under Settings, then Appearance, then Font file.
- The Windows installer is not signed, so Windows SmartScreen says "Windows protected your PC" when you run it. Choose More info, then Run anyway. On a Windows 11 PC where Smart App Control is on, Windows can block the installer and Cairn outright, with no Run anyway.
- If you built Cairn 1.2.0 or older from source on Windows, it kept the notebook folder with a `\\?\` prefix, and this version keeps it without, unless the folder is on a network share or its path is very long. For a notebook opened before, it then lists the folder a second time under the recent notebooks, does not reopen its tabs, keeps its plugins off until you turn them on again, and has no sync set up until you connect again. Connecting again matches the notes on this computer to the server's by path and content, so a note that is the same on both sides is not uploaded twice. A note that changed on either side since this computer last synced gets a conflict copy, and a note deleted on another device comes back, so connect again before you edit notes on this computer.

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
