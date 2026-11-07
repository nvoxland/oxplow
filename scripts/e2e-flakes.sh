#!/bin/sh
# Which browser specs failed or flaked on CI lately, and how fast the suite
# ran: `bun run e2e:flakes [runs=20]`. Reads the `e2e-junit` artifact of the
# last runs of ci.yml on main (`gh`), and prints, per spec, how many runs it
# failed or needed its retry in; then each run's wall time, the sum of its
# tests' times and the average number of busy workers.
set -eu
RUNS=${1:-20}
DIR=$(mktemp -d)
trap 'rm -rf "$DIR"' EXIT
gh run list --workflow ci.yml --branch main --limit "$RUNS" --json databaseId,headSha \
  -q '.[] | "\(.databaseId) \(.headSha[0:9])"' | while read -r id sha; do
  mkdir -p "$DIR/$id"
  gh run download "$id" -n e2e-junit -D "$DIR/$id" >/dev/null 2>&1 || rmdir "$DIR/$id"
  [ -d "$DIR/$id" ] && echo "$sha" > "$DIR/$id/sha"
done
python3 -I "$(dirname "$0")/e2e-flakes.py" "$DIR"
