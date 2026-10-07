# Windows

## Install

The Windows installer, `Cairn_<version>_x64-setup.exe` on the [releases page](https://github.com/lucas-pospor/cairn/releases), is for Windows 10 and 11 on x86_64.

It is not signed, so Windows SmartScreen says "Windows protected your PC" when you run it: choose More info, then Run anyway. On a Windows 11 PC where Smart App Control is on, there is no Run anyway: Windows blocks unsigned apps that it does not know, so it can block the installer and Cairn outright.

The installer installs Cairn for your user account, without administrator rights, by default in `%LOCALAPPDATA%\Cairn` (you can choose another folder). Cairn needs the WebView2 Runtime, which Windows 11 and an up-to-date Windows 10 already have; if it is missing, the installer downloads it from Microsoft.

## App data and uninstalling

Cairn keeps its own files (the recent notebooks, plugin approvals and sync state) in `%APPDATA%\app.cairn.notes`, and the data of its window, such as each notebook's open tabs, in `%LOCALAPPDATA%\app.cairn.notes`. Uninstalling leaves both unless you tick "Delete the application data", and never touches your notebooks.

## File names

Windows does not allow some names that other systems do: names with `< > : " | ? *`, names that end in a dot or a space, and device names such as CON, NUL, COM1 or LPT1, with any extension. A file with such a name from another device does not sync to Windows; Settings, then Sync, lists it under "Files not synced".

The [known limits](known-limits.md#windows) list what works differently on Windows. To build Cairn on Windows, see [Build on Windows](building.md#build-on-windows).
