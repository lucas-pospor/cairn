# Links and embeds

## Wikilinks

Type `[[` to link to a note or another file, and autocomplete lists the notes and other files in the notebook. A link can show other text (`[[note|text]]`), lead to a heading (`[[note#heading]]`), or name a path: `./` and `../` are relative to the note the link is in. A link to a block (`[[note#^id]]`) opens the note but does not go to the block; an embed of it (`![[note#^id]]`) shows the block. Clicking a link to a missing note creates it. Alt+Enter follows the link at the cursor. Ctrl+click opens a link in a new tab, and so does a middle click in the editor.

A link finds its note by the exact path in the notebook (`.md` is optional), then by a path relative to the note the link is in, then by the note's name. Case does not matter in any of these steps. A link that names folders, such as `[[Projects/Plan]]`, finds by name only a note whose path ends in those folders, so it does not open `Archive/Plan.md`. When several notes have the name, the one in the same folder as the linking note wins, then the one with the shortest path.

Standard Markdown links to `.md` files in the notebook count as links too. They look first for the path relative to the note the link is in, then follow the same steps as a wikilink.

## Embeds

`![[note]]` and `![[note#heading]]` embed a note or one of its sections. Images (`![[photo.jpg|300]]`), audio and video embed too. A small text file (up to 256 KB) embeds as text; any other file embeds as a card with its name.

## Attachments

Paste or drop images and other files into a note. Cairn stores them in the notebook's attachment folder and links them. [Notebooks and files](notebooks-and-files.md#images) describes how images and other files open.
