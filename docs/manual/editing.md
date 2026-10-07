# Editing

## Live Preview

In Live Preview, Markdown renders as you type, and the syntax comes back on the line you are editing. Source mode, reading view and a side-by-side split each take one click, and Ctrl+E switches between editing and reading.

## Tabs and autosave

Notes open in tabs and save themselves as you type. If a file changes on disk while you have unsaved edits, Cairn merges the change into your edits when the two change different lines that are not next to each other. If they touch the same or neighboring lines, or either side rewrote more than about 10,000 lines (lines only added or only removed in one place do not count), it stops and asks instead of overwriting either version.

Edits that Cairn could not save (a conflict, a read-only file, a failed write) stay in their tab, and closing the tab, switching notebooks or closing the window asks first.

## Panels

The right sidebar shows a note's backlinks, its outgoing links, its outline, its properties (frontmatter) and its tags. The Tags panel in the left sidebar lists every tag in the notebook with the number of notes that use it; click a tag to search for it.

## Word count

The status bar shows a word and character count, for the note or for the selection. Chinese and Japanese text counts each Han, Hiragana or Katakana character as a word. Thai, Lao, Khmer and Myanmar text is split into words with the web view's dictionary, which differs a little between Linux, Windows and Android.

## Hotkeys

Every command can have a hotkey, and Settings, then Hotkeys, lists them all. A hotkey needs Ctrl, Alt or Cmd unless it is a function key. It goes by the character the key types in your keyboard layout (AZERTY, Dvorak and so on).

Keyboard shortcuts use Cmd instead of Ctrl on macOS.

[Core plugins](core-plugins.md) adds templates and daily notes, and [Themes and appearance](themes-and-appearance.md) describes the fonts, the font size and zoom.
