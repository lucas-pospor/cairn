# Plugins

A plugin is a JavaScript file in `.cairn/plugins/` inside the notebook folder. Turn it on under Settings, then Plugins, on each device where it should run. Each plugin runs in its own sandbox (a Web Worker) with no access to the page, the network or the file system. It talks to Cairn only through the `cairn` object, and only with the permissions it declares at the top of the file.

## An example

```js
// @name Selection stats
// @description Counts the words in the selection.
// @permissions editor
cairn.commands.register("count", "Count selected words", async () => {
  const text = (await cairn.editor.getSelection()) ?? "";
  await cairn.ui.toast(`${text.split(/\s+/).filter(Boolean).length} words`);
});
```

## The API

- `cairn.commands.register(id, name, handler)` and `cairn.ui.toast(text)`: always allowed.
- `cairn.notes.list()` and `cairn.notes.read(path)`: permission `read`.
- `cairn.notes.write(path, content)`: permission `write`.
- `cairn.editor.activePath()`, `cairn.editor.getSelection()` and `cairn.editor.replaceSelection(text)`: permission `editor`. They act on the note on screen and give null or false while no note is shown. In the reading view, `replaceSelection` gives false, and `getSelection` gives the selection the note last had in the editor.

`notes.read` and `notes.write` reach only Markdown notes outside dot-folders, and never leave the notebook folder through a symlink.

A plugin can register up to 200 commands, which appear in the command palette and can get hotkeys. A command still running after 2 seconds shows a notice. If a plugin's command runs for more than 30 seconds, or the plugin stops responding or floods the app with messages, Cairn stops the plugin and turns it off. Plugins have no memory limit, so a plugin that keeps allocating memory can make the window go blank.

## Turning a plugin on

Turning a plugin on records an approval on that device, in `plugin-approvals.json` in Cairn's configuration folder (`~/.config/app.cairn.notes/` on Linux, `%APPDATA%\app.cairn.notes\` on Windows), never in the notebook folder. On Windows with a roaming profile, Windows copies that folder to the other computers you sign in to, so the approval holds there too for a notebook at the same path (see the [known limits](known-limits.md#windows)). The approval holds a hash of the plugin file and the permissions you approved, so a change to the file, or moving the notebook folder, turns the plugin off until you turn it on again.

A plugin that the notebook lists but this device has not approved stays off, and a toast says so when you open the notebook. Only `.js` files directly in `.cairn/plugins/` can run, and Settings, then Plugins, lists them. A plugin file larger than 16 MB cannot run. It can leave that list empty, and if the notebook's settings turn it on, it can keep other plugins from starting. Remove the file or make it smaller (see the [known limits](known-limits.md#files-and-links)).
