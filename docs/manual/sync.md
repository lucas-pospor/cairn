# Sync and the sync server

Cairn syncs a notebook between your devices through a small server you run yourself. Sync is end-to-end encrypted, keeps conflict copies instead of losing edits, and keeps a version history of each note. Sync is optional: Cairn works without it.

Sync does not copy files or folders whose names start with a dot. So the `.cairn` folder does not sync, and settings, CSS snippets, plugins and the font file stay on each device. The [known limits](known-limits.md#sync) list what else sync does not do yet.

## What the server can see

The server stores notes and file names only in encrypted form. Each device encrypts them with a random notebook key that only your passphrase unlocks, so the server never sees the content or the name of any file. The server does see the notebook name, the name of each device (set in the setup form, where it starts as the host name on Linux, the computer's name on Windows, or "Android" on a phone), how many files the notebook has, which changes belong to the same file, and the size, upload time and kind of each change.

## Run a server

### With Docker Compose and Caddy

The repository's `crates/cairn-server` folder has a Compose file and a Caddyfile. The Compose file builds the server image from the source code, so use it in a clone of the repository. Caddy gets an HTTPS certificate for the server by itself when the DNS name is public and the machine can be reached from the internet on port 80 or 443.

1. Point a DNS name at the machine and put it in `crates/cairn-server/Caddyfile`.
2. Create `crates/cairn-server/.env` containing `CAIRN_TOKENS=` followed by a long random secret (for example the output of `openssl rand -hex 24`).
3. Start it:

```bash
cd crates/cairn-server && docker compose up -d
```

### With Docker alone

Without Compose, build and run the image directly (from the repository root):

```bash
docker build -f crates/cairn-server/Dockerfile -t cairn-server .
```

```bash
docker run -d --name cairn -p 8787:8787 -v cairn-data:/data -e CAIRN_TOKENS=your-secret cairn-server
```

To use the image from a release instead of building it, load it with `docker load -i cairn-server-1.4.1-docker-image.tar.gz` and use `cairn-server:1.4.1` as the image name. That image is for x86_64 machines. On another machine, such as a Raspberry Pi, build the image as above.

### Without Docker

Run the binary with `cargo run --release -p cairn-server` and `CAIRN_TOKENS` set.

### Settings

The server reads its settings from environment variables: `CAIRN_TOKENS` (required, comma-separated), `CAIRN_DATA` (database folder, default `./data`), `CAIRN_ADDR` (default `0.0.0.0:8787`), `CAIRN_MAX_BODY_MB` (default 200). Cairn sends and reads at most 200 MB per request, and files travel base64-encoded, so the largest file that syncs is about 150 MB. A lower `CAIRN_MAX_BODY_MB` lowers that limit, but a higher one does not raise it.

### HTTPS and reverse proxies

Put the server behind HTTPS. The content is encrypted either way, but the access token is not. Cairn accepts only certificates from the public certificate authorities built into it, so a self-signed certificate, or one from a local authority such as Caddy's own, does not work. On a local network with no public name, the server can only be reached over plain `http://`, where the token is not encrypted.

The server closes a connection that sends no request for 10 seconds. The Caddyfile in the repository keeps idle connections to the server for 5 seconds. If you use another reverse proxy, set its idle timeout for connections to the server below 10 seconds as well. The server logs each request that has a missing or wrong token, together with the client's address (behind a reverse proxy, that is the proxy's address).

## Set up each device

On each device, open Settings, then Sync, and enter the server URL, the token, a notebook name and the passphrase. Use the same notebook name and passphrase everywhere. If the server has no notebook with that name, Cairn asks "There's no notebook called X on this server. Create it?" and creates it only if you say yes, so a mistyped name does not start a second, empty notebook. If the URL points at a wrong path or at another web server, setup stops with "server error: there is no Cairn sync server at this address. Check the URL".

Cairn needs the passphrase only to connect a notebook folder on a device: the device then keeps the notebook key for that folder and syncs without the passphrase. If you lose the passphrase, your connected devices keep syncing, but you cannot connect another device to the notebook. You also cannot connect a device again after you turn sync off on it, move or rename its notebook folder, or lose Cairn's app data on it, for example by uninstalling the Android app. Once no device is connected, nobody can decrypt the data on the server, not even you. The notes on your devices are not affected.

## When sync runs

Sync runs a few seconds after you stop typing, every minute, and when you click the sync status in the status bar. If a sync you start yourself (by clicking the sync status, or with Sync now in the command palette) fails, a message says why.

## Conflicts

If both devices change a note before they sync and the changes touch the same or neighboring lines, or either side rewrote more than about 10,000 lines, Cairn keeps both. It saves the other version next to yours as "Note (conflict 2026-10-03 1530 laptop).md", with the date and time in UTC and the name of the device that made the copy, and lists it under "Conflict copies" in Settings, then Sync.

If one device renames a note, or the folder it is in, and another deletes the note, Cairn keeps the note under the new name.

## Renames, deleted files and version history

A note renamed or moved in Cairn keeps its version history even when you edit it before the next sync. A note renamed outside Cairn and edited before the next sync counts as deleted and new: its history starts again, and the other devices move the old note to their trash. Deleted files go to the trash (`.trash/` in the notebook folder on Android). On Windows, what the Recycle Bin cannot take, such as a file on a network share or a USB stick, goes to `.trash/` in the notebook folder too. If Windows still deletes a file for good instead of moving it to the Recycle Bin, Settings, then Sync, lists it under "Files not synced" with an error that says so, until the next sync.

With sync on, "Version history" in a file's menu in the file tree ("Show version history of current note" in the command palette) shows earlier versions of a note and can restore them. Restoring syncs first, so the text it replaces stays in the history. If that sync fails, Cairn restores nothing.

## Files not synced

A file that cannot sync does not hold up the others. The status bar then says "Synced · 1 file not synced", and Settings, then Sync, lists each such file under "Files not synced" with its reason:

- too large, refused or stalled on upload;
- unreadable here, or not decryptable;
- not writable on this device, for example a name that Windows does not allow, or a name that the phone's shared storage does not allow, that differs only in case from another file there, or that is in a folder the phone has under another spelling;
- a change the server no longer has;
- a name with a backslash;
- a change to a file that this device reaches under two names through a symlink (see [Files under two names](notebooks-and-files.md#files-under-two-names)), or that may be another name of a file in a folder this device cannot read now;
- a move that can break a symlink.

A file drops off the list once it syncs.

## When sync stops

In two cases sync stops without uploading anything, and the sync status shows "Sync error" with the reason in its tooltip and in Settings, then Sync.

The first is a notebook folder with no files at all on a device that has synced before, for example a moved folder or an unmounted drive. Files in hidden folders such as `.cairn` and `.trash` do not count. If you deleted every note on purpose, add a note and sync again.

The second is a server that has lost changes this device has seen, or no longer has the notebook. In that case, turn sync off in Settings, then Sync, and connect again.

## Connecting again

Connecting again, or losing the sync state on a device, does not duplicate notes: Cairn matches the device's files to the server's by path and content. It does not delete anything at that point, so a note deleted on one side comes back everywhere. On a first connect, if a new file's content matches a server note this device lacks, Cairn takes the file as that note, and the note then moves to the file's path on the other devices. This happens only when the match is one to one, and never for empty files.
