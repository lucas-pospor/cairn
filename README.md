<p align="center">
  <img src="app/src-tauri/icons/icon.svg" width="112" alt="Cairn logo: a stack of stones">
</p>

<h1 align="center">Cairn</h1>

Cairn is a local-first Markdown notes app for Linux, Windows and Android. A notebook is an ordinary folder of `.md` files: Cairn reads and writes those files directly, so you can open the same folder in any other editor, put it under Git, or back it up however you like. Sync between your devices is optional and end-to-end encrypted, through a small server you run yourself.

The website, [Cairn Notes](https://lucas-pospor.github.io/cairn/), has the downloads and the manual. The manual is also here, in [docs/manual](docs/manual/README.md): it describes how to use Cairn, and its [known limits](docs/manual/known-limits.md) page lists what Cairn does not do yet. [docs/RELEASE_NOTES.md](docs/RELEASE_NOTES.md) describes each release, and [docs/PLAN.md](docs/PLAN.md) has the design, the stack decisions and the status of each version.

## Screenshots

![Cairn on Linux: a note in Live Preview, with the file tree on the left and the note's backlinks on the right](docs/images/editor.png)

| Graph view (Slate theme) | Full-text search |
|:---:|:---:|
| ![The graph view in the Slate theme, with one note and its links highlighted](docs/images/graph.png) | ![Search results for "tomato" next to the open note](docs/images/search.png) |

<p align="center">
  <img src="docs/images/android-editor.png" width="270" alt="A note on Android, with the formatting toolbar">
  &nbsp;&nbsp;
  <img src="docs/images/android-files.png" width="270" alt="The Files drawer on Android">
</p>

## Manual

The manual's pages, in [docs/manual](docs/manual/README.md):

- [Getting started](docs/manual/getting-started.md)
- [Notebooks and files](docs/manual/notebooks-and-files.md)
- [Editing](docs/manual/editing.md)
- [Links and embeds](docs/manual/links-and-embeds.md)
- [Search and graph](docs/manual/search-and-graph.md)
- [Themes and appearance](docs/manual/themes-and-appearance.md)
- [Sync and the sync server](docs/manual/sync.md)
- [Core plugins](docs/manual/core-plugins.md)
- [Plugins](docs/manual/plugins.md)
- [Android](docs/manual/android.md)
- [Windows](docs/manual/windows.md)
- [Building from source](docs/manual/building.md)
- [Known limits](docs/manual/known-limits.md)

## Building

Prebuilt packages are on the [releases page](https://github.com/lucas-pospor/cairn/releases). To build Cairn yourself, see [Building from source](docs/manual/building.md).

## Tests

Core logic (file operations, change detection, link parsing and resolution, search):

```bash
cargo test -p cairn-core
```

The sync server has its own tests:

```bash
cargo test -p cairn-server
```

So does the Rust code of the app itself (`app/src-tauri`, package `cairn`). Run its unit tests with `--lib`, which builds only the tests and leaves the built app alone:

```bash
cargo test -p cairn --lib
```

UI logic (tree building, link helpers, Markdown rendering):

```bash
cd app && npm test
```

Type-check the UI:

```bash
cd app && npm run check
```

End-to-end tests drive the real app through WebDriver, so they need `tauri-driver` (the tests look for it in `~/.cargo/bin`, or set `TAURI_DRIVER`) and, on Linux, `WebKitWebDriver` (shipped with WebKitGTK on most distributions; Debian and Ubuntu package it as `webkit2gtk-driver`). WebDriver testing is not available on macOS, and the desktop end-to-end tests are run on Linux. On Windows, a run uses your own Cairn settings in `%APPDATA%\app.cairn.notes`: its test notebooks join your recent notebooks and are gone afterwards, so the next normal start opens the Welcome screen.

```bash
cargo install tauri-driver --locked
cargo build -p cairn-server
cd app && npm run e2e:build && npm run e2e
```

The tests create a throwaway notebook in the system temp folder and check the results on disk. `e2e/sync.test.mjs` also starts a sync server (`target/debug/cairn-server`) and uses the `sync_dir` example as a second device. `npm run e2e:build` builds the app and the `sync_dir` example but not the server binary, so build that with `cargo build -p cairn-server` as shown.

Run the Rust tests per crate, with `-p`. A plain `cargo build`, or `cargo test` at the workspace root, relinks `target/debug/cairn` without the UI built in. That binary expects the Vite dev server, so the end-to-end tests stop working until you run `npm run e2e:build` again.

On Linux, to run end-to-end test files without windows on your desktop, or several runs at once, use `scripts/e2e-headless.sh` from the repository root. It needs `mutter`: each run gets its own headless Wayland display (`mutter --headless`) and its own WebDriver ports (`CAIRN_WD_PORT` and `CAIRN_WD_NATIVE_PORT`). It passes its arguments to `node --test`:

```bash
scripts/e2e-headless.sh e2e/app.test.mjs e2e/sync.test.mjs
```

Two more desktop test files are in `scripts/`. `npm run e2e` does not run them, so use the headless runner:

```bash
scripts/e2e-headless.sh scripts/adv-fs-e2e.test.mjs scripts/adv-links-e2e.test.mjs
```

Sync engine tests, including two simulated devices that edit, rename and delete concurrently through a real server, and a randomized test that checks that the devices converge without losing any edit (`CAIRN_FUZZ_SEEDS` sets how many random runs):

```bash
cargo test -p cairn-sync
```

Without `CAIRN_FUZZ_SEEDS`, that is 8 or 30 runs, depending on the test. After a change to sync, run more, for example 200:

```bash
CAIRN_FUZZ_SEEDS=200 cargo test -p cairn-sync
```

Android end-to-end tests run on an emulator or phone through adb (they clear the app's data). The test files share the device, so run them one at a time, from the repository root after `. scripts/android-env.sh`:

```bash
node --test --test-concurrency=1 e2e/android/*.test.mjs
```

They need a debug APK (`cd app && npx tauri android build --debug --apk --target x86_64` for the emulator), the `sync_dir` example from `npm run e2e:build` and the server binary from `cargo build -p cairn-server`.

The emulator's memory use grows over long runs. `scripts/adv-android-run-all.sh` runs each test file on a freshly booted emulator instead, using an AVD named `cairn-test`. It runs every file in `e2e/android/`, or the files you name, writes logs to `e2e/.tmp/android-run/` and prints a summary at the end:

```bash
scripts/adv-android-run-all.sh e2e/android/adv_saf.test.mjs
```

`e2e/android/local_network.test.mjs` checks the Nearby devices permission (see [Sync on the local network](docs/manual/android.md#sync-on-the-local-network)). It needs an emulator with Android 17 (API 37) and skips on older ones. With an AVD named `cairn-test-37` made from the `system-images;android-37.0;google_apis;x86_64` image:

```bash
AVD=cairn-test-37 scripts/adv-android-run-all.sh e2e/android/local_network.test.mjs
```

To check it on a phone with Android 17 by hand: run a sync server on a computer in the same Wi-Fi network and note the computer's address (for example `192.168.1.20`). Install the APK. If Cairn has or had the Nearby devices permission, reset it with `adb shell pm revoke app.cairn.notes android.permission.ACCESS_LOCAL_NETWORK` and `adb shell pm clear-permission-flags app.cairn.notes android.permission.ACCESS_LOCAL_NETWORK user-set user-fixed`: after a removal in Android settings alone, Android takes the first Don't allow as final. Open Settings, then Sync. Enter `http://192.168.1.20:8787` (or the server's address and port), fill in the rest and press Connect and sync. Android shows its prompt for nearby devices. Press Don't allow: within a few seconds, not after 30, the sync error names the server, says it is on your local network and names the setting. Press Connect and sync again and Allow: the notebook syncs. Then remove the permission in Android settings and open Cairn again: Settings, then Sync, shows "Press Sync now to allow it", no prompt shows by itself, and Sync now shows the prompt.

## Performance testing

Generate a large notebook and time how long the core takes to open and query it:

```bash
node scripts/gen-vault.mjs /tmp/big-notebook 10000
```

```bash
cargo run --release -p cairn-core --example bench_open -- /tmp/big-notebook
```

The app logs how long it took from process start to the first painted screen (`UI ready ... ms after process start`).

## Project layout

```
crates/cairn-core/   notebook (Vault in the code), file system abstraction, parser, index, search (Rust, no UI)
crates/cairn-sync/   sync protocol, encryption, merge rules, client engine
crates/cairn-server/ sync server (axum + SQLite), Dockerfile, compose file, Caddyfile
app/src-tauri/       Tauri shell: commands, file watcher, sync thread, vault:// protocol
app/src-tauri/gen/android/  Android project; SafPlugin.kt is the Storage Access Framework bridge, LocalNetworkPlugin.kt asks for the local network permission
app/src/             Svelte UI and CodeMirror extensions
app/src/lib/corePlugins/  core plugins (Templates, Daily notes, Unique note creator, Random note)
e2e/                 end-to-end tests against the built app (WebDriver on the desktop, adb on Android)
scripts/             build, test and benchmark helpers
docs/manual/         the user manual, also on the website
site/                the website: page templates, styles and build script
docs/PLAN.md         plan, decisions, status
docs/RELEASE_NOTES.md  what is in each release
docs/images/         screenshots used in the README, the manual and the website
LICENSE              MIT license
```

## License

MIT (see [LICENSE](LICENSE)).
