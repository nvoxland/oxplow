#!/usr/bin/env bash
# The headless binaries never build the desktop shell's stack (tsk885).
#
#   scripts/check-headless-graph.sh
#
# The daemon runs where there's no display (CI installs no WebKit for it)
# and the providers ship on their own, so none of them may reach Tauri,
# WebKit, GTK or AppKit through any dependency, the workspace-hack
# included. Fails listing each crate that does, on each platform the hack
# is generated for.
set -euo pipefail

crates=(oxplow-daemon oxplow-rpc oxplow-provider-fake oxplow-provider-linear oxplow-provider-mcp)
targets=(aarch64-apple-darwin x86_64-unknown-linux-gnu)
desktop='^(tauri|tauri-runtime|wry|webkit2gtk|webkit2gtk-sys|gtk|gtk-sys|objc2-app-kit|objc2-web-kit) '

failed=0
for target in "${targets[@]}"; do
  for crate in "${crates[@]}"; do
    found=$(cargo tree --quiet -p "$crate" -e normal --target "$target" --prefix none \
      | sed "s/ (\*)$//" | grep -E "$desktop" | sort -u || true)
    if [ -n "$found" ]; then
      echo "$crate ($target) builds the desktop stack:" >&2
      echo "$found" | sed 's/^/  /' >&2
      failed=1
    fi
  done
done
exit "$failed"
