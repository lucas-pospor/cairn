#!/usr/bin/env bash
# Run Android e2e test files one at a time, each on a freshly booted emulator
# (the emulator's host memory grows over long runs). Needs the debug APK
# (cd app && npx tauri android build --debug --apk --target x86_64) and the
# server/sync_dir binaries from `npm run e2e:build`.
#
#   scripts/adv-android-run-all.sh                     # every file in e2e/android/
#   scripts/adv-android-run-all.sh e2e/android/adv_saf.test.mjs
#
# Logs go to e2e/.tmp/android-run/<file>.log; a summary is printed at the end.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
. scripts/android-env.sh
set -uo pipefail
APK=app/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
OUT=e2e/.tmp/android-run
mkdir -p "$OUT"
files=("$@")
[ ${#files[@]} -gt 0 ] || files=(e2e/android/*.test.mjs)

summary=()
for f in "${files[@]}"; do
  name=$(basename "$f" .test.mjs)
  emulator -avd cairn-test -no-window -no-audio -gpu guest -memory 2048 -no-snapshot >"$OUT/$name.emulator.log" 2>&1 &
  emu=$!
  adb wait-for-device
  until [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ]; do sleep 2; done
  adb shell settings put system font_scale 1.0 >/dev/null
  adb shell settings put system accelerometer_rotation 0 >/dev/null
  adb shell settings put system user_rotation 0 >/dev/null
  adb shell cmd uimode night no >/dev/null
  adb install -r "$APK" >/dev/null
  timeout 3600 node --test --test-concurrency=1 "$f" >"$OUT/$name.log" 2>&1
  code=$?
  summary+=("$name exit=$code $(grep -E '^ℹ (tests|pass|fail|todo|skipped)' "$OUT/$name.log" | awk '{printf "%s=%s ", $2, $3}')")
  adb emu kill >/dev/null 2>&1
  sleep 5
  kill "$emu" 2>/dev/null
  wait "$emu" 2>/dev/null
done
printf '%s\n' "${summary[@]}"
