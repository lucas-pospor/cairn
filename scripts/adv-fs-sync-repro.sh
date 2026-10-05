#!/usr/bin/env bash
# Reproductions for three cairn-core file-level defects (FINDING-012,
# FINDING-011, FINDING-049) that would spread to other devices through sync,
# using the prebuilt server and the sync_dir example as two devices.
# Everything lives in a temp folder that is removed at the end.
#
#   scripts/adv-fs-sync-repro.sh
#
# Prints one "REPRODUCED" or "held up" line per scenario. Exit code is the
# number of scenarios that reproduced.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SERVER="$ROOT/target/debug/cairn-server"
SYNC="$ROOT/target/debug/examples/sync_dir"
T="$(mktemp -d /tmp/cairn-advfs-sync-XXXXXX)"
TOKEN="advfs-token-$$"
PORT=$((20000 + RANDOM % 20000))
URL="http://127.0.0.1:$PORT"

CAIRN_TOKENS="$TOKEN" CAIRN_DATA="$T/server" CAIRN_ADDR="127.0.0.1:$PORT" "$SERVER" >"$T/server.log" 2>&1 &
SERVER_PID=$!
cleanup() { kill "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null; rm -rf "$T"; }
trap cleanup EXIT
trap 'exit 130' INT TERM HUP PIPE
up=0
for _ in $(seq 1 150); do curl -sf "$URL/health" >/dev/null 2>&1 && { up=1; break; }; sleep 0.1; done
[ "$up" = 1 ] || { echo "server did not start:"; cat "$T/server.log"; exit 100; }

dev() { # dev <vault> <device> <vault-id>
  "$SYNC" "$T/$1" "$T/state-$1" "$URL" "$TOKEN" "$3" "$2" "pass phrase" >"$T/$1.json" 2>"$T/$1.err" || { echo "  sync of $1 failed: $(cat "$T/$1.err")"; }
}
list() { (cd "$T/$1" && find . -path ./.trash -prune -o -type f -print | sort | sed 's|^\./||'); }
fails=0

echo "== FINDING-012: one symlink loop on device A multiplies every note on device B"
mkdir -p "$T/a1"
echo "my only note" >"$T/a1/note.md"
ln -s . "$T/a1/loop"
dev a1 laptop v-loop
dev b1 phone v-loop
n=$(list b1 | wc -l)
echo "  device A has 1 real file; device B received $n files, deepest: $(list b1 | awk '{ print length($0), $0 }' | sort -n | tail -1 | cut -d' ' -f2- | cut -c1-70)..."
if [ "$n" -gt 1 ]; then echo "  REPRODUCED"; fails=$((fails + 1)); else echo "  held up"; fi

echo "== FINDING-011: a note with an NFD name (copied from a Mac) never syncs"
mkdir -p "$T/a2"
printf 'hello from a mac\n' >"$T/a2/$(printf 'cafe\xcc\x81.md')"
echo "plain" >"$T/a2/plain.md"
dev a2 laptop v-nfd
dev b2 phone v-nfd
echo "  device B has: $(list b2 | tr '\n' ' ')"
if ! list b2 | grep -q 'caf'; then echo "  REPRODUCED (NFD note silently not uploaded)"; fails=$((fails + 1)); else echo "  held up"; fi

echo "== FINDING-011: replacing a synced note by its NFD-named copy deletes it on the other device"
mkdir -p "$T/a3"
printf 'v1\n' >"$T/a3/$(printf 'caf\xc3\xa9.md')"
dev a3 laptop v-nfd2
dev b3 phone v-nfd2
before=$(list b3 | grep -c caf)
# e.g. the folder is restored from a Mac backup: same note, NFD file name
rm "$T/a3/$(printf 'caf\xc3\xa9.md')"
printf 'v1 plus an edit\n' >"$T/a3/$(printf 'cafe\xcc\x81.md')"
dev a3 laptop v-nfd2
dev b3 phone v-nfd2
after=$(list b3 | grep -c caf)
echo "  device B had $before copy, now has $after (trash: $(ls "$T/b3/.trash" 2>/dev/null | tr '\n' ' '))"
if [ "$before" = 1 ] && [ "$after" = 0 ]; then echo "  REPRODUCED"; fails=$((fails + 1)); else echo "  held up"; fi

echo "== FINDING-049: an unreadable subfolder stops sync for the whole vault"
mkdir -p "$T/a4/private"
echo "n" >"$T/a4/n.md"
dev a4 laptop v-perm
chmod 000 "$T/a4/private"
echo "new note" >"$T/a4/new.md"
"$SYNC" "$T/a4" "$T/state-a4" "$URL" "$TOKEN" v-perm laptop "pass phrase" >"$T/a4.json" 2>"$T/a4.err"
rc=$?
chmod 755 "$T/a4/private"
echo "  sync exit $rc: $(head -c 200 "$T/a4.err")"
if [ "$rc" != 0 ]; then echo "  REPRODUCED"; fails=$((fails + 1)); else echo "  held up"; fi

exit "$fails"
