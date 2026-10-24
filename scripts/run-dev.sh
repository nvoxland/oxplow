#!/bin/sh
# Build oxplow from source, deploy its extensions, and run it: the whole
# dev loop after a core change, in one command (`bun run run:dev`).
#
#   bun run run:dev                 # this checkout, as the project
#   bun run run:dev ~/src/other     # another project
#   bun run run:dev --no-deploy     # build and run; deploy nothing
#
# 1. Refuses while oxplow holds the project (its instance lock), before
#    the build: the deploy needs oxplow quit, and so does a clean start.
# 2. `bun run build:dev` — stages the sidecars, builds the frontend and
#    the shell (`DEV.md` "Run from source").
# 3. `oxplow-dev extension deploy` — deploys every extension that is on
#    and whose source changed, oxplow-foundation first. Boot deploys
#    nothing by itself, so a core change that foundation's source changed
#    with needs this before oxplow starts. Nothing changed: each already
#    runs, and it says so.
# 4. Runs `target/debug/oxplow <project>` in the foreground, with this
#    shell's environment (`OXPLOW_HOME` and the rest pass through).
#
# An extension change alone needs none of this: deploy it in the running
# app (`oxplow.extension.deploy`, or Settings → Extensions).
set -e
repo="$(cd "$(dirname "$0")/.." && pwd)"

deploy=1
project=""
for arg in "$@"; do
  case "$arg" in
    --no-deploy) deploy=0 ;;
    -h | --help)
      sed -n '2,/^set -e/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    -*)
      echo "run-dev: unknown option $arg (see --help)" >&2
      exit 2
      ;;
    *) project="$arg" ;;
  esac
done
project="$(cd "${project:-$repo}" && pwd)"

lock="$project/.oxplow/instance.lock"
if [ -e "$lock" ] && holder="$(lsof -t "$lock" 2>/dev/null)" && [ -n "$holder" ]; then
  echo "run-dev: oxplow is running on $project (pid $(echo $holder)); quit it first." >&2
  exit 1
fi

cd "$repo"
echo "run-dev: building" >&2
bun run build:dev

if [ "$deploy" = 1 ]; then
  echo "run-dev: deploying extensions" >&2
  cargo run -q -p oxplow-dev -- --project "$project" extension deploy
fi

echo "run-dev: starting oxplow on $project" >&2
exec "$repo/target/debug/oxplow" "$project"
