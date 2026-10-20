#!/bin/sh
# What the test and lint runs need that no crate's own graph builds —
# a fresh worktree (or one after `cargo clean`) otherwise fails its first
# run on them:
#
# - the sidecars, built and restaged every time (a no-op when current):
#   tauri-build validates `externalBin` from oxplow-desktop's build
#   script, so any build that compiles the desktop crate (a workspace
#   test or clippy run) fails without them; and it copies each staged
#   one over `target/debug/<name>` when that build script runs, which is
#   where the tests look for the script worker, so a staged copy older
#   than the code ran old code under the tests (the "scope() is
#   available in command handlers only" failures across every test that
#   runs a script) and put an old daemon under the app;
# - the fake provider's binary, which oxplow-app's provider tests spawn
#   (a `-p oxplow-app` run doesn't build another crate's bins), built
#   every time for the same reason: a stale fake tests yesterday's
#   protocol against today's host.
#
# It first keeps `target/` bounded (`sweep-target.sh`, at most every 30
# minutes), so whatever the sweep drops is rebuilt in the same run.
set -e
cd "$(dirname "$0")/.."
sh scripts/sweep-target.sh
bash apps/desktop/src-tauri/scripts/stage-sidecars.sh debug >&2
cargo build -q -p oxplow-provider-fake >&2
