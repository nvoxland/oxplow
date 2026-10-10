#!/bin/sh
# What the test and lint runs need that no crate's own graph builds,
# built only when missing — a fresh worktree (or one after `cargo clean`)
# otherwise fails its first run on them:
#
# - the staged sidecars: tauri-build validates `externalBin` from
#   oxplow-desktop's build script, so any build that compiles the desktop
#   crate (a workspace test or clippy run) fails without them;
# - the fake provider's binary, which oxplow-app's provider tests spawn
#   (a `-p oxplow-app` run doesn't build another crate's bins);
# - the script worker every Starlark and jaq run spawns: built every time
#   (a no-op when current), since a stale one runs old script code — and
#   restaged: tauri-build copies the staged sidecar over
#   `target/debug/oxplow-script-worker` whenever oxplow-desktop's build
#   script runs, which is where the tests look for it, so a staged copy
#   older than the fresh build put a stale worker under the first run
#   after any change to the script crate (the "scope() is available in
#   command handlers only" failures across every test that runs a script).
#
# It first keeps `target/` bounded (`sweep-target.sh`, at most every 30
# minutes), so whatever the sweep drops is rebuilt in the same run.
set -e
cd "$(dirname "$0")/.."
sh scripts/sweep-target.sh
triple="$(rustc -vV | awk '/^host: / { print $2 }')"
if [ ! -f "apps/desktop/src-tauri/binaries/oxplow-daemon-$triple" ] ||
  [ ! -f "apps/desktop/src-tauri/binaries/oxplow-script-worker-$triple" ]; then
  bash apps/desktop/src-tauri/scripts/stage-sidecars.sh debug >&2
fi
# Always built (a no-op when current): a stale fake or worker would test
# yesterday's protocol against today's host.
cargo build -q -p oxplow-provider-fake >&2
cargo build -q -p oxplow-script --bin oxplow-script-worker >&2
# The staged worker is the fresh one, so the copy tauri-build makes of it
# (when oxplow-desktop's build script next runs, after this) is current.
worker="target/debug/oxplow-script-worker"
staged="apps/desktop/src-tauri/binaries/oxplow-script-worker-$triple"
if [ -f "$worker" ] && [ -f "$staged" ] && ! cmp -s "$worker" "$staged"; then
  cp "$worker" "$staged" >&2
fi
