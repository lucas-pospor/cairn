#!/usr/bin/env bash
# Run desktop e2e test files on a private headless Wayland display
# (mutter --headless), so no windows open on your desktop and several runs can
# go at the same time. Each run takes a free "slot" with its own display and
# WebDriver ports.
#
#   scripts/e2e-headless.sh e2e/app.test.mjs e2e/adv_dataloss.test.mjs
#   scripts/e2e-headless.sh --test-name-pattern 'conflict' e2e/app.test.mjs
#
# Arguments are passed to `node --test --test-concurrency=1`.
# Build the app first: cd app && npm run e2e:build
set -euo pipefail


BASE="${CAIRN_E2E_RUNDIR:-/tmp/cairn-e2e-$(id -u)}"
mkdir -p "$BASE"

slot=""
for n in $(seq 1 24); do
  exec {lockfd}>"$BASE/slot$n.lock"
  if flock -n "$lockfd"; then slot=$n; break; fi
  exec {lockfd}>&-
done
[ -n "$slot" ] || { echo "no free e2e slot" >&2; exit 1; }

run="$BASE/s$slot"
rm -rf "$run"; mkdir -p "$run"
display="cairn-e2e-$slot"
XDG_RUNTIME_DIR="$run" mutter --headless --wayland --no-x11 --wayland-display "$display" \
  --virtual-monitor 1280x800 >"$run/mutter.log" 2>&1 &
mutter_pid=$!
trap 'kill $mutter_pid 2>/dev/null || true' EXIT
for _ in $(seq 1 100); do [ -S "$run/$display" ] && break; sleep 0.1; done
[ -S "$run/$display" ] || { echo "mutter did not start:"; cat "$run/mutter.log"; exit 1; } >&2


env -u DISPLAY WAYLAND_DISPLAY="$run/$display" GDK_BACKEND=wayland \
  CAIRN_WD_PORT=$((14440 + slot * 10)) CAIRN_WD_NATIVE_PORT=$((14441 + slot * 10)) \
  node --test --test-concurrency=1 "$@"
