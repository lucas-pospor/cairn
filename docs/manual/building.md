# Building from source

Prebuilt packages are on the [releases page](https://github.com/lucas-pospor/cairn/releases). This page describes how to build Cairn yourself.

## Requirements

To build Cairn from source, you need:

- Rust 1.90 or newer (`rustup` or your distribution's package)
- Node.js 22.12 or newer and npm (Vitest runs on Node 22, 24, and 26 or later)
- The Tauri system dependencies for your platform, listed at <https://tauri.app/start/prerequisites/>

Platform notes:

- Linux: WebKitGTK 4.1 and its development headers. On Debian or Ubuntu: `sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`. On Arch: `sudo pacman -S webkit2gtk-4.1 base-devel curl wget file openssl appmenu-gtk-module libappindicator-gtk3 librsvg`.
- macOS: Xcode Command Line Tools (`xcode-select --install`). Nobody has built or run Cairn on macOS yet, CI does not test it there, and releases have no macOS build.
- Windows: Microsoft C++ Build Tools and the WebView2 Runtime. Clone the repository as described under [Build on Windows](#build-on-windows), below.

## Build the desktop app

Get the source code with Git. On Windows, clone it as described under [Build on Windows](#build-on-windows), below. The clone makes a `cairn` folder: run the other commands on this page in that folder.

```bash
git clone https://github.com/lucas-pospor/cairn.git
cd cairn
```

Install the JavaScript dependencies once:

```bash
cd app && npm install
```

Run in development mode, with hot reload for the UI:

```bash
cd app && npx tauri dev
```

Build installable packages for the current OS (Linux: `.deb`, `.rpm` and AppImage; macOS: `.app` and `.dmg`; Windows: `.msi` and `.exe`):

```bash
cd app && npx tauri build
```

The packages end up in `target/release/bundle/`.

## Build on Windows

To build Cairn on Windows, clone the repository as CI does, with LF line endings kept and long paths allowed:

```bash
git clone -c core.autocrlf=false -c core.longpaths=true https://github.com/lucas-pospor/cairn.git
```

Then build as above, in Command Prompt, Git Bash or PowerShell 7: Windows PowerShell 5.1, which comes with Windows, does not accept the `&&` in these commands. `cd app && npx tauri build --bundles nsis` builds only the installer, as CI does.

## Build the Android app

The Android app is built from the same code. In addition to the [requirements](#requirements) above, you need:

- `rustup` with the Android targets: `rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android` (a distribution's Rust package cannot add targets)
- JDK 17 or newer
- The Android SDK with platform 37, build tools, the NDK and platform tools. With the command-line tools: `sdkmanager "platform-tools" "platforms;android-37.0" "build-tools;36.0.0" "ndk;29.0.14206865"`

`scripts/android-env.sh` sets `ANDROID_HOME`, `NDK_HOME`, `JAVA_HOME` and `PATH` for the default locations; edit it if yours differ.

```bash
. scripts/android-env.sh
```

Build an APK for phones (arm64) and for the x86_64 emulator:

```bash
cd app && npx tauri android build --apk --target aarch64 --target x86_64
```

For a debug build on a connected phone or a running emulator:

```bash
cd app && npx tauri android dev
```

You have to sign a release APK before you can install it. See <https://tauri.app/distribute/sign/android/>.
