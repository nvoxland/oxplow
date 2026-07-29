#!/usr/bin/env bash
# Build `oxplow-daemon` and stage it where Tauri's `externalBin` expects
# it, i.e. `binaries/oxplow-daemon-<target-triple>` (tsk256).
#
# The packaged shell spawns one daemon per open project and resolves the
# binary next to its own executable — which is exactly where a sidecar
# lands inside `Oxplow.app/Contents/MacOS/`.
#
# Locates everything from its own path, so it doesn't care where it's
# invoked from. It is invoked from `beforeBuildCommand`, whose cwd is the
# **app dir** (`apps/desktop`) — not `src-tauri`, which cost tsk263 a
# packaged build with no daemon in it.
set -euo pipefail

cd "$(dirname "$0")/.."
repo_root="$(cd ../../.. && pwd)"
triple="$(rustc -vV | awk '/^host: / { print $2 }')"

# Windows binaries carry `.exe`, and Tauri looks for the sidecar under
# `<name>-<triple>.exe` — the extension goes after the triple, not before.
ext=""
case "$triple" in
*windows*) ext=".exe" ;;
esac

cargo build --release -p oxplow-daemon --manifest-path "$repo_root/Cargo.toml"

mkdir -p binaries
cp "$repo_root/target/release/oxplow-daemon$ext" "binaries/oxplow-daemon-$triple$ext"
echo "staged binaries/oxplow-daemon-$triple$ext"
