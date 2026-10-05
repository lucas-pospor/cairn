#!/usr/bin/env bash
# Kill the Android emulator (by PID) if its resident memory or the machine's
# available memory crosses a limit. The emulator's host-side GPU emulation
# (-gpu swiftshader_indirect) was seen growing to 18 GB RSS during long e2e
# runs, which is enough to run a 32 GB machine out of memory.
#
#   scripts/adv-android-watchdog.sh <emulator-pid> [max-rss-mb] [min-available-mb]
#
# Runs until the emulator exits. Logs to stdout.
pid="$1"; max_rss="${2:-6000}"; min_avail="${3:-3000}"
[ -n "$pid" ] || { echo "usage: $0 <emulator-pid> [max-rss-mb] [min-available-mb]" >&2; exit 2; }
while kill -0 "$pid" 2>/dev/null; do
  rss=$(( $(awk '/^VmRSS:/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0) / 1024 ))
  avail=$(awk '/^MemAvailable:/{print int($2/1024)}' /proc/meminfo)
  if [ "$rss" -gt "$max_rss" ] || [ "$avail" -lt "$min_avail" ]; then
    echo "$(date -Iseconds) watchdog: emulator rss ${rss} MB, available ${avail} MB -> killing emulator $pid"
    kill "$pid"; sleep 5; kill -9 "$pid" 2>/dev/null
    exit 1
  fi
  sleep 5
done
echo "$(date -Iseconds) watchdog: emulator $pid exited"
