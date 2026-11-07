#!/bin/sh
# Run one spec (or a grep) many times at once, the way a race fix is
# confirmed: `bun run e2e:flake settings/pages.spec.ts [times]`.
# Each repeat lands in the JUnit report, so oxplow counts its flips.
set -eu
[ $# -ge 1 ] || { echo "usage: bun run e2e:flake <spec path or pattern> [times=10]" >&2; exit 2; }
SPEC=$1
TIMES=${2:-10}
exec bunx playwright test "$SPEC" --project chromium --repeat-each "$TIMES" --workers 4
