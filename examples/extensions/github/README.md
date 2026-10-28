# GitHub pull requests: an example oxplow extension

Pulls your repo's 100 most recent pull requests into oxplow as `v_github_pr`,
and adds a **Pull Requests by Work Item** page that matches each PR to the
work item its title mentions by id (`tsk42`, `ENG-12` — whichever work list
is active), and a **PR Lifetimes** page that draws
each one as a bar from when it was opened to when it was merged.

A pull request is also a resource a URI can name: `oxplow:github/pr?id=12`,
or `[[pr:12]]` in a task, a comment or a wiki page. It shows the pull
request's title, opens its page, and is found by search.

## Use it

1. Copy this folder to `.oxplow/extensions/github/` in your repo (or point
   `extensionSources` at a clone), then deploy it with
   `oxplow.extension.deploy { name: "github" }`.
2. Make sure `jq` is installed, and either `gh` is logged in or you set
   `GITHUB_TOKEN` under the `github` extension in Settings → Extensions (it's
   stored in your keychain).
   The deploy asks you to approve what `extension.yaml` declares under
   `permissions:`: it runs programs (`jq`, `gh`, `git`), reaches
   `api.github.com` and reads `GITHUB_TOKEN`.
3. Settings → Data → **Sync Now** on the `github/prs` source.

It re-syncs every 15 minutes after that. **PR Lifetimes** is a custom
component: its Sync button runs that sync with your rights, under the
extension's permissions, so it works as soon as the extension is deployed.
The repo comes from `git remote get-url origin`, or set
`GITHUB_REPOSITORY=owner/name`.

## Files

- `extension.yaml`: the `prs` source and the `pr` entity's columns.
- `sync.sh`: fetches from the GitHub API and reshapes the result with `jq`.
- `pages/prs-by-task.yaml`: the page.
- `pages/pr-lifetimes.yaml`, `components/pr-lifetimes/`: the lifetimes page
  and the bundle it renders (`custom_components:` in `extension.yaml`).
- `datasets/pull_request.sql`, `pages/pr.yaml`: what a `github/pr` URI
  resolves to, and the page it opens (`resources:` in `extension.yaml`).
- `fixtures/pr-link.yaml`: `oxplow plugin test`'s check that `[[pr:12]]`
  names `oxplow:github/pr?id=12`.

See the [guide to pages](../../../docs/guide/lenses.md) for the format.
