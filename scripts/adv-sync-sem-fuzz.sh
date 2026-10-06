#!/usr/bin/env bash
# Run the randomized three-device sync fuzz (crates/cairn-sync/tests/
# adv_sync_semantics_fuzz.rs) over many seeds and list the failing ones.
#
#   scripts/adv-sync-sem-fuzz.sh [seq|overlap] [seeds] [first seed] [steps] [threads]
#
#   seq      syncs never overlap (must pass)            default
#   overlap  another device syncs in the middle of a sync, and the user saves
#            notes while a sync runs (must pass)
#
# Examples:
#   scripts/adv-sync-sem-fuzz.sh seq 300
#   scripts/adv-sync-sem-fuzz.sh overlap 1000 0 160 6
#   CAIRN_SS_FUZZ_LOG=1 scripts/adv-sync-sem-fuzz.sh overlap 1 9     # op log of seed 9
set -euo pipefail
cd "$(dirname "$0")/.."
mode=${1:-seq}
export CAIRN_SS_FUZZ_SEEDS=${2:-300}
export CAIRN_SS_FUZZ_START=${3:-0}
export CAIRN_SS_FUZZ_STEPS=${4:-160}
export CAIRN_SS_FUZZ_THREADS=${5:-4}
case "$mode" in
  seq) exec cargo test -p cairn-sync --test adv_sync_semantics_fuzz randomized_three_device_rich_ops -- --exact --nocapture ;;
  overlap) exec cargo test -p cairn-sync --test adv_sync_semantics_fuzz randomized_three_device_rich_ops_overlapping_syncs -- --exact --nocapture ;;
  *) echo "mode must be seq or overlap" >&2; exit 2 ;;
esac
