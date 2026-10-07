# Core plugins

Core plugins are optional features that come with Cairn: Templates, Daily notes, Unique note creator and Random note. Each one has a switch under Settings, then Core plugins, and some have options of their own, which show under the switch while the plugin is on. You can turn all of them off: only their own commands and buttons go away.

They are part of the app, so they need no approval on each device, unlike [plugins](plugins.md) in `.cairn/plugins/`. None of them does anything when you open a notebook: they act only when you run one of their commands from the command palette or press one of their buttons. Their commands have no default hotkeys, but you can give them hotkeys under Settings, then Hotkeys. A hotkey stays saved while its plugin is off.

## Templates

On by default. Templates inserts a note from the template folder (`Templates` unless you choose another) at the cursor. In the template, `{{title}}` becomes the name of the note, and `{{date}}` and `{{time}}` become the date and time in the formats set in Settings (`YYYY-MM-DD` and `HH:mm` unless you change them). `{{date:FORMAT}}` and `{{time:FORMAT}}` use a format of their own. The template goes in as it is, frontmatter included, and one undo takes it out again. On a phone, a button in the formatting toolbar inserts a template.

## Daily notes

On by default. Daily notes opens today's note, named by the date (`YYYY-MM-DD` unless you change it) in the folder you choose (the notebook folder itself unless you change it). If there is no note for today yet, Cairn creates one, starting from a template note if you set one. A slash in the format makes folders, as in `YYYY/MM/YYYY-MM-DD`. Its button sits next to Graph view at the top of the left sidebar, and on a phone at the top of the Files drawer.

## Unique note creator

Off by default. Unique note creator creates a note named by the date and time (`YYYYMMDDHHmm` unless you change it), in the folder you choose and from a template note if you set one, and opens it in a new tab.

## Random note

Off by default. Random note opens a note picked at random.

## Date formats

Dates use the format letters of moment.js: for example `YYYY` (year), `MM` (month), `DD` (day), `dddd` (weekday), `MMMM` (month name), `Do` (day with an ending such as 5th), `HH:mm` (time), `ww` and `WW` (week numbers) and `[text]` for text that stays as it is. Day and month names are always English, so devices with different system languages give a note the same name. Settings shows what a format gives today, or why the name it makes cannot be a file name.

## Your notes are never overwritten

Core plugins never write over a note. A daily note that is already there opens as it is. If a note appears at that name while Cairn creates it (sync or another app made it, or its name differs only in case), Cairn opens that note instead. A unique note whose name is taken gets a number, such as `202610051432 1.md`. Templates change only the note you are editing, through the editor and its autosave.

## Where the settings are

The switches and options are stored in `.cairn/settings.json`, under `corePlugins`, so they belong to the notebook like the other settings. Cairn sync does not copy `.cairn/`, so with Cairn sync you turn the plugins on and set them up on each device. Only the values you change are written, and Cairn keeps anything under `corePlugins` that it does not know. Version 1.0.0 keeps the key too when it saves other settings.
