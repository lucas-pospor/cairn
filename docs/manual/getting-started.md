# Getting started

Cairn is a local-first Markdown notes app. A notebook is an ordinary folder of `.md` files: Cairn reads and writes those files directly, so you can open the same folder in any other editor, put it under Git, or back it up however you like. The search index and link graph live in memory, and Cairn rebuilds them from the files.

## Install

Prebuilt Linux packages, a Windows installer, the Android APK and the sync server's Docker image are on the [releases page](https://github.com/lucas-pospor/cairn/releases). [Windows](windows.md) describes the installer and [Android](android.md) the APK. There is no macOS build. To build Cairn yourself, see [Building from source](building.md).

## Open a notebook

Open any folder as a notebook, or create a new one. If you type a path where there is no folder, Cairn asks before it creates a notebook there (`~` means your home folder). Cairn remembers recent notebooks.

You can pass a notebook folder on the command line or set `CAIRN_VAULT`. Without either, Cairn reopens the last notebook. On Linux, the .deb and .rpm packages add the `cairn` command, as in `cairn ~/Notes`; the AppImage takes the folder the same way, as in `./Cairn_<version>_amd64.AppImage ~/Notes`. Cairn does not expand `~` in a path given on the command line or in `CAIRN_VAULT`, and the Windows installer does not add Cairn to the PATH, so on Windows give full paths, as in `"%LOCALAPPDATA%\Cairn\cairn.exe" "%USERPROFILE%\Notes"` in Command Prompt.

## Next steps

- [Notebooks and files](notebooks-and-files.md) describes the file tree and how Cairn handles the files in a notebook.
- [Editing](editing.md) and [Links and embeds](links-and-embeds.md) describe writing notes.
- [Sync and the sync server](sync.md) describes how to sync a notebook between devices.
