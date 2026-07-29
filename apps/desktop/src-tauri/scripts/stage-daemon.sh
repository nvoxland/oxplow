#!/usr/bin/env bash
# Build `oxplow-daemon` and stage it where Tauri's `externalBin` expects
# it, i.e. `binaries/oxplow-daemon-<target-triple>` (tsk256).
#
# The packaged shell spawns one daemon per open project and resolves the
# binary next to its own executable — which is exactly where a sidecar
# lands inside `Oxplow.app/Contents/MacOS/`.
#
# **This is not only a packaging step.** `externalBin` makes tauri-build
# validate the sidecar from oxplow-desktop's *build script*, so `cargo
# test`, `cargo clippy --all-targets` and a bare `cargo build -p
# oxplow-desktop` all fail without it (tsk266). Anything that compiles
# the desktop crate needs this to have run once.
#
#   ./stage-daemon.sh            # release — what a bundle ships
#   ./stage-daemon.sh debug      # debug — enough to satisfy the build
#                                #   script, and shares the dependency
#                                #   build with a debug workspace build
#
# Locates everything from its own path, so it doesn't care where it's
# invoked from. It is invoked from `beforeBuildCommand`, whose cwd is the
# **app dir** (`apps/desktop`) — not `src-tauri`, which cost tsk263 a
# packaged build with no daemon in it.
set -euo pipefail

profile="${1:-release}"
case "$profile" in
release) profile_flag="--release"; target_subdir="release" ;;
debug) profile_flag=""; target_subdir="debug" ;;
*)
  echo "stage-daemon: unknown profile '$profile' (want 'release' or 'debug')" >&2
  exit 2
  ;;
esac

cd "$(dirname "$0")/.."
repo_root="$(cd ../../.. && pwd)"
triple="$(rustc -vV | awk '/^host: / { print $2 }')"

# Windows binaries carry `.exe`, and Tauri looks for the sidecar under
# `<name>-<triple>.exe` — the extension goes after the triple, not before.
ext=""
case "$triple" in
*windows*) ext=".exe" ;;
esac

# shellcheck disable=SC2086 # profile_flag is intentionally word-split (empty for debug)
cargo build $profile_flag -p oxplow-daemon --manifest-path "$repo_root/Cargo.toml"

mkdir -p binaries
cp "$repo_root/target/$target_subdir/oxplow-daemon$ext" "binaries/oxplow-daemon-$triple$ext"
echo "staged binaries/oxplow-daemon-$triple$ext ($profile)"
