#!/usr/bin/env bash
# Sign a debug binary with the local dev identity (DEV.md "Sign dev builds"),
# so the keychain trusts every rebuild as the same app: after one Always
# Allow it stops asking. Two binaries read the keychain: the daemon (the AI
# keys) and the shell (the device tokens for hosts on other machines).
#
#   bash scripts/sign-dev.sh <binary> <identifier>
#
# Does nothing off macOS, without the identity, or when the binary is signed
# by it already: a signature records its time, so signing twice would change
# the bytes (and restage and relink) on every run.
set -euo pipefail

binary="$1"
identifier="$2"
identity="${OXPLOW_DEV_SIGNING_IDENTITY:-oxplow dev}"
[ "$(uname)" = Darwin ] || exit 0
security find-identity -p codesigning 2>/dev/null | grep -qF "\"$identity\"" || exit 0
if ! codesign -dvv "$binary" 2>&1 | grep -qxF "Authority=$identity"; then
  codesign --force --sign "$identity" --identifier "$identifier" --timestamp=none "$binary"
  echo "signed $binary ($identity)"
fi
