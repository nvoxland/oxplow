#!/bin/sh
# What the test and lint runs need that no crate's own graph builds,
# built only when missing — a fresh worktree (or one after `cargo clean`)
# otherwise fails its first run on them:
#
# - the staged sidecars: tauri-build validates `externalBin` from
#   oxplow-desktop's build script, so any build that compiles the desktop
#   crate (a workspace test or clippy run) fails without them;
# - the fake provider's binary, which oxplow-app's provider tests spawn
#   (a `-p oxplow-app` run doesn't build another crate's bins).
#
# It first keeps `target/` bounded (`sweep-target.sh`, at most every 30
# minutes), so whatever the sweep drops is rebuilt in the same run.
set -e
cd "$(dirname "$0")/.."
sh scripts/sweep-target.sh
triple="$(rustc -vV | awk '/^host: / { print $2 }')"
if [ ! -f "apps/desktop/src-tauri/binaries/oxplow-daemon-$triple" ]; then
  bash apps/desktop/src-tauri/scripts/stage-sidecars.sh debug >&2
fi
if [ ! -x target/debug/oxplow-provider-fake ]; then
  cargo build -q -p oxplow-provider-fake >&2
fi
