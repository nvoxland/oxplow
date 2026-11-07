#!/bin/sh
# Keep `target/` bounded. Cargo never deletes an artifact: every dependency
# change gives a crate a new hash and a fresh incremental cache beside the
# old one, so a busy worktree grew from ~23 GB of working set to 59 GB of
# mostly dead copies in a day (13 incremental caches of oxplow-app alone,
# 8 of them unused since the morning's merges), and three worktrees filled
# the disk. cargo-sweep drops artifacts of toolchains no longer installed,
# then the oldest until `target/` is under the cap (incremental caches,
# which cargo-sweep doesn't count, are trimmed to each crate's newest ten). The
# cap has to stay above one worktree's working set as cargo-sweep counts it, a
# full test + clippy + coverage + binary build plus the stale copies that gather
# between sweeps: 64 GiB by a day's work, measured with `cargo sweep --maxsize
# <cap> --dry-run`. Above the cap it removes the oldest first, and the oldest are the third-party
# dependencies, which are live and the slowest to build: a 30 GB cap made a run
# rebuild them every 30 minutes (470 of a full run's 1,068 s). When the disk
# runs short, `cargo clean` a worktree that isn't in use.
#
#   scripts/sweep-target.sh          # at most every 30 minutes (the test
#                                    # and lint runs call it this way)
#   scripts/sweep-target.sh --now    # now (`bun run clean:target`)
cd "$(dirname "$0")/.."
CAP=80GB
STAMP=target/.sweep-stamp
if ! command -v cargo-sweep >/dev/null 2>&1; then
  echo "sweep-target: cargo-sweep isn't installed, so target/ grows unbounded (cargo install cargo-sweep --locked)" >&2
  exit 0
fi
if [ "$1" != "--now" ] && [ -f "$STAMP" ] && [ -z "$(find "$STAMP" -mmin +30)" ]; then
  exit 0
fi
mkdir -p target
# Incremental caches first: cargo-sweep doesn't count them, and each
# dependency change starts a crate a new one (`<crate>-<hash>`) beside the
# old — 808 of them, 65 GB, in one worktree. So keep the newest ten per crate per
# profile. A crate uses several at once, one each for its library, its tests,
# `check` and clippy (oxplow-app: 8): keeping two made the next edit's build
# start cold, 200 s against 19 s, each time this ran.
for dir in target/*/incremental; do
  [ -d "$dir" ] || continue
  ls -t "$dir" | awk '{ crate = $0; sub(/-[^-]*$/, "", crate); if (++seen[crate] > 10) print }' |
    while read -r stale; do rm -rf "${dir:?}/$stale"; done
done
cargo sweep --installed >/dev/null 2>&1
cargo sweep --maxsize "$CAP" 2>&1 | grep -i "cleaned" >&2
touch "$STAMP"
exit 0
