# GitHub pull requests: an example oxplow extension

Pulls your repo's 100 most recent pull requests into oxplow as `v_github_pr`,
and adds a **Pull Requests by Task** lens that matches each PR to the oxplow
task its title mentions (`tsk42`).

## Use it

1. Copy this folder to `oxplow/extensions/github/` in your repo (or publish it
   as its own repo and install it from Settings → Extensions).
2. Make sure `jq` is installed, and either `gh` is logged in or you set
   `GITHUB_TOKEN` under the `github` extension in Settings → Extensions (it's
   stored in your keychain).
3. Settings → Data → **Approve & Run** on the `github/prs` source.

It re-syncs every 15 minutes after that. The repo comes from `git remote get-url
origin`, or set `GITHUB_REPOSITORY=owner/name`.

## Files

- `extension.yaml`: the `prs` source and the `pr` entity's columns.
- `sync.sh`: fetches from the GitHub API and reshapes the result with `jq`.
- `lenses/prs-by-task.yaml`: the lens.

See the [Lenses guide](../../../docs/guide/lenses.md) for the format.
