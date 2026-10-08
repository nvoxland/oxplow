#!/bin/sh
# Keep `target/` bounded. Cargo never deletes an artifact: every dependency
# change gives a crate a new hash and a fresh incremental cache beside the
# old one, so a busy worktree grew from ~23 GB of working set to 59 GB of
# mostly dead copies in a day (13 incremental caches of oxplow-app alone,
# 8 of them unused since the morning's merges), and three worktrees filled
# the disk. cargo-sweep drops artifacts of toolchains no longer installed,
# then the oldest until `target/` is under the cap — above one worktree's
# working set (a full test + clippy + binary build, ~23 GB), so what's in
# use stays.
#
#   scripts/sweep-target.sh          # at most every 30 minutes (the test
#                                    # and lint runs call it this way)
#   scripts/sweep-target.sh --now    # now (`bun run clean:target`)
cd "$(dirname "$0")/.."
CAP=30GB
STAMP=target/.sweep-stamp
if ! command -v cargo-sweep >/dev/null 2>&1; then
  echo "sweep-target: cargo-sweep isn't installed, so target/ grows unbounded (cargo install cargo-sweep --locked)" >&2
  exit 0
fi
if [ "$1" != "--now" ] && [ -f "$STAMP" ] && [ -z "$(find "$STAMP" -mmin +30)" ]; then
  exit 0
fi
mkdir -p target
cargo sweep --installed >/dev/null 2>&1
cargo sweep --maxsize "$CAP" 2>&1 | grep -i "cleaned" >&2
touch "$STAMP"
exit 0
