#!/bin/sh
# oxplow source: this repo's recent pull requests as `pr` entities.
#
# Needs `jq`, plus either GITHUB_TOKEN (uses curl) or a logged-in `gh`.
# Runs from the extension folder inside the repo, so `git remote` finds
# the repo unless GITHUB_REPOSITORY (owner/name) is set.
set -eu

repo="${GITHUB_REPOSITORY:-$(git remote get-url origin | sed -E 's#^(git@github\.com:|https://github\.com/)##; s#\.git$##')}"
path="repos/$repo/pulls?state=all&sort=updated&direction=desc&per_page=100"

if [ -n "${GITHUB_TOKEN:-}" ]; then
  json=$(curl -fsSL \
    -H "Authorization: Bearer $GITHUB_TOKEN" \
    -H "Accept: application/vnd.github+json" \
    "https://api.github.com/$path")
else
  json=$(gh api "$path")
fi

printf '%s' "$json" | jq '{entities: {pr: [.[] | {
  number,
  title,
  state,
  author: .user.login,
  head_branch: .head.ref,
  draft,
  opened_at: .created_at,
  merged_at,
  url: .html_url
}]}}'
