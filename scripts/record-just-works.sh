#!/usr/bin/env bash
# Record a fresh agent building an extension of one kind with nothing but
# the oxplow-extension skill (P7.C6, `.context/extensions.md` "The SDK").
#
#   scripts/record-just-works.sh <kind>
#
# Reads crates/oxplow-sdk/fixtures/just-works/<kind>/prompt.md, runs
# `claude -p` on it in an empty git project — --safe-mode, so no CLAUDE.md,
# plugins, hooks, memory or MCP: only the skill, appended to its system
# prompt, and the `oxplow` CLI built from this checkout — then writes, next
# to the prompt:
#   run.json   the agent's run (claude --output-format json), without
#              what's of this machine or session: the denied commands
#              (local paths), the session id and the cost
#   produced/  the extension folder(s) it wrote
#   check.txt  `oxplow plugin check` on each, after the run
#   test.txt   `oxplow plugin test` on each
# Write notes.md by hand: what it took, what the agent tripped on.
# `recorded_agent_runs_still_check_and_test_clean` replays produced/.
set -euo pipefail

kind="${1:?usage: scripts/record-just-works.sh <kind>}"
repo="$(cd "$(dirname "$0")/.." && pwd)"
fixture="$repo/crates/oxplow-sdk/fixtures/just-works/$kind"
prompt="$fixture/prompt.md"
[ -f "$prompt" ] || { echo "no $prompt" >&2; exit 2; }

cargo build --quiet --manifest-path "$repo/Cargo.toml" -p oxplow-desktop --bin oxplow
export PATH="$repo/target/debug:$PATH"

project="$(mktemp -d)"
trap 'rm -rf "$project"' EXIT
git -C "$project" init -q
git -C "$project" -c user.name=record -c user.email=record@localhost commit -q --allow-empty -m init

(
  cd "$project"
  claude -p "$(cat "$prompt")" \
    --safe-mode \
    --no-session-persistence \
    --append-system-prompt-file "$repo/crates/oxplow-agent-text/assets/oxplow-extension.SKILL.md" \
    --permission-mode acceptEdits \
    --allowedTools Read Write Edit Glob Grep "Bash(oxplow plugin:*)" "Bash(ls:*)" "Bash(cat:*)" "Bash(mkdir:*)" \
    --output-format json
) | jq -c 'del(.permission_denials, .session_id, .uuid, .total_cost_usd, .modelUsage)' \
  > "$fixture/run.json"

rm -rf "$fixture/produced"
mkdir -p "$fixture/produced"
cp -R "$project/oxplow/extensions/." "$fixture/produced/"

: > "$fixture/check.txt"
: > "$fixture/test.txt"
for dir in "$fixture/produced"/*/; do
  name="$(basename "$dir")"
  (cd "$project" && oxplow plugin check "$name" || true) >> "$fixture/check.txt" 2>&1
  (cd "$project" && oxplow plugin test "$name" || true) >> "$fixture/test.txt" 2>&1
done
echo "recorded $kind: $(ls "$fixture/produced")"
