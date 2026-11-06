#!/usr/bin/env bash
# Wait for CI's `test` and `e2e` jobs on a commit and say whether they passed.
# A release builds only a commit CI has passed, so it has no test steps of its
# own to drift from CI's. The tag may be pushed while CI is still running.
#
#   scripts/ci-passed.sh <sha> [poll seconds]
set -euo pipefail

sha="${1:?usage: ci-passed.sh <sha> [poll seconds]}"
poll="${2:-60}"

while :; do
  run=$(gh run list --workflow CI --commit "$sha" --event push --limit 1 --json databaseId --jq '.[0].databaseId // empty')
  if [ -n "$run" ]; then
    jobs=$(gh run view "$run" --json jobs --jq '.jobs[] | select(.name == "test" or .name == "e2e") | "\(.name) \(.status) \(.conclusion)"')
    echo "CI run $run:"
    echo "$jobs"
    if [ "$(echo "$jobs" | grep -c ' completed ')" = 2 ]; then
      if [ "$(echo "$jobs" | grep -c ' completed success$')" = 2 ]; then
        exit 0
      fi
      echo "::error::CI's test and e2e did not both pass on $sha (run $run). Re-run them, then re-run this job."
      exit 1
    fi
  else
    echo "No CI run for $sha yet."
  fi
  sleep "$poll"
done
