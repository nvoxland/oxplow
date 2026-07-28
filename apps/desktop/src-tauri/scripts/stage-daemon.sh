#!/usr/bin/env bash
# Build `oxplow-daemon` and stage it where Tauri's `externalBin` expects
# it, i.e. `binaries/oxplow-daemon-<target-triple>` (tsk256).
#
# The packaged shell spawns one daemon per open project and resolves the
# binary next to its own executable — which is exactly where a sidecar
# lands inside `Oxplow.app/Contents/MacOS/`. Run from `src-tauri` (the
# `beforeBuildCommand` cwd).
set -euo pipefail

cd "$(dirname "$0")/.."
repo_root="$(cd ../../.. && pwd)"
triple="$(rustc -vV | awk '/^host: / { print $2 }')"

cargo build --release -p oxplow-daemon --manifest-path "$repo_root/Cargo.toml"

mkdir -p binaries
cp "$repo_root/target/release/oxplow-daemon" "binaries/oxplow-daemon-$triple"
echo "staged binaries/oxplow-daemon-$triple"
