#!/usr/bin/env bash
# Build the sidecars and stage them where Tauri's `externalBin` expects
# them, i.e. `binaries/<name>-<target-triple>`:
#
# - `oxplow-daemon` (tsk256): the packaged shell spawns one per open
#   project;
# - `oxplow-provider-mcp` (P7.A6): oxplow's MCP adapter, which the daemon
#   runs for a provider declared with `adapter:`;
# - `oxplow-script-worker`: the process every Starlark and jaq script runs
#   in, so a runaway one can be killed (`oxplow_script::workers`).
#
# All are resolved next to the running executable — which is exactly
# where a sidecar lands inside `Oxplow.app/Contents/MacOS/`.
#
# **This is not only a packaging step.** `externalBin` makes tauri-build
# validate the sidecar from oxplow-desktop's *build script*, so `cargo
# test`, `cargo clippy --all-targets` and a bare `cargo build -p
# oxplow-desktop` all fail without it (tsk266). Anything that compiles
# the desktop crate needs this to have run once.
#
#   ./stage-sidecars.sh          # release — what a bundle ships
#   ./stage-sidecars.sh debug    # debug — enough to satisfy the build
#                                #   script, and shares the dependency
#                                #   build with a debug workspace build
#
# **Stage again whenever a sidecar's crate changed.** tauri-build copies
# each staged file next to the shell (`target/<profile>/<name>`) when
# oxplow-desktop's build script runs, over the one cargo just built: a
# staged copy older than the code puts that old binary under the app,
# and under the tests that spawn it. `bun run build:dev` and the test and
# lint runs (`scripts/test-prereqs.sh`) stage first.
#
# Locates everything from its own path, so it doesn't care where it's
# invoked from. It is invoked from `beforeBuildCommand`, whose cwd is the
# **app dir** (`apps/desktop`) — not `src-tauri`, which cost tsk263 a
# packaged build with no daemon in it.
sidecars=(oxplow-daemon oxplow-provider-mcp oxplow-script-worker)
set -euo pipefail

profile="${1:-release}"
case "$profile" in
release) profile_flag="--release"; target_subdir="release" ;;
debug) profile_flag=""; target_subdir="debug" ;;
*)
  echo "stage-sidecars: unknown profile '$profile' (want 'release' or 'debug')" >&2
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
cargo build $profile_flag -p oxplow-daemon -p oxplow-provider-mcp -p oxplow-script --bins --manifest-path "$repo_root/Cargo.toml"

# Copied only when it changed: tauri-build reruns oxplow-desktop's build
# script when a staged file is touched, so an unconditional copy would
# relink the desktop on every run.
mkdir -p binaries
for name in "${sidecars[@]}"; do
  built="$repo_root/target/$target_subdir/$name$ext"
  staged="binaries/$name-$triple$ext"
  if ! cmp -s "$built" "$staged"; then
    cp "$built" "$staged"
    echo "staged $staged ($profile)"
  fi
done
