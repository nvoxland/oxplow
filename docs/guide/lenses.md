# Lenses

A lens is a page you (or your agent) build: a SQL query over oxplow's data
plus how to show it. It lives as a small YAML file in your repo, shows up in
the launcher (++cmd+p++, under **Lenses**), and re-runs as the data changes.

The quickest way to get one is to ask your agent: "make me a lens of the
tasks blocked in this stream". It knows the format.

## The data

Everything oxplow tracks is exposed as read-only SQL views: `v_task`,
`v_effort`, `v_thread`, `v_comment`, `v_wiki_page`, `v_effort_file`,
`v_agent_turn`, `v_token_usage`, `v_fact`, and more.

**Explore Data** (launcher → Data) lists every view with a description of each
column. Pick one to see sample rows, edit the SQL (++cmd+enter++ runs it), and
**Save as Lens** when it shows what you want.

## Writing one by hand

Lenses live in `oxplow/extensions/<extension>/lenses/<slug>.yaml`. The
extension folder needs an `extension.yaml`:

```yaml
# oxplow/extensions/review/extension.yaml
name: review          # must match the folder name
description: Lenses for reviewing agent work
```

```yaml
# oxplow/extensions/review/lenses/blocked.yaml
title: Blocked Tasks
params:
  - { name: stream, label: Stream, default: 1 }
query: |
  SELECT id, title, updated_at FROM v_task
  WHERE status = 'blocked' AND stream_id = :stream
viz: table            # table | list | number | markdown
columns:
  - { key: title, label: Task, link: { kind: task, from: id } }
  - { key: updated_at, label: Updated }
empty: Nothing is blocked.
```

- `params` become inputs on the page and bind as `:name`.
- `link.kind` is `task`, `file`, `wiki` or `effort-diff`.
- Unknown keys are errors, so typos show up instead of being ignored.

Problems show under Settings → Extensions. Agents check their work with
`validate_extension`.

## On the page

- Right-click a row → **Add Row to Agent Context**.
- **Improve with Agent** puts `[oxplow lens <id>]` in the agent's input so you
  can ask for changes.
- **Pin to Dashboard** adds it as a [dashboard](dashboards.md) tile.
- The agent can see what you're looking at: `get_open_page` returns the lens
  with the exact rows on your screen.

## Sharing

- **Your team:** `oxplow/extensions/` is ordinary project files. Commit it.
- **Anyone:** put an extension in its own git repo (with `extension.yaml` at
  the root) and install it from Settings → Extensions → paste the git URL.
  **Update** pulls the latest. Don't edit installed extensions in place;
  updates overwrite them.

## Sources: bringing in outside data

An extension can declare a **source**: a script that fetches records from
somewhere else (GitHub, Linear, your CI) and prints them as JSON. Oxplow
stores them as a view like `v_github_pr` that lenses can join with tasks and
efforts.

```yaml
sources:
  - id: prs
    runtime: exec
    entry: sync.sh               # inside the extension folder
    schedule: every 15m          # or: manual
    env: [GITHUB_REPOSITORY]     # passed through from your environment
    credentials: [GITHUB_TOKEN]  # secrets from your keychain
    entities:
      - name: pr
        key: number
        columns: { number: int, title: text, state: text, merged_at: time }
```

The script prints:

```json
{"entities": {"pr": [{"number": 12, "title": "Fix login", "state": "open", "merged_at": null}]}}
```

Column types are `text`, `int`, `real`, `bool` and `time` (an ISO timestamp).

A source runs code, so **nothing runs until you approve it** in Settings →
Extensions → **Approve & Run**. Approval is per machine and per version of the
script: a teammate who pulls it approves it themselves, and a changed script
needs approving again. Agents can run approved sources but can't approve
them.

Known limits:

- It only gets the environment variables it declares, plus `PATH` and `HOME`.
  Network access isn't restricted yet.
- Secrets go in `credentials:`. Set each one under the source in Settings →
  Extensions; the value goes to your OS keychain and the script gets it as
  that environment variable. Values are per extension, and agents can't
  read or set them.
- Sources run from the primary stream's worktree. Their data is shared by
  every stream.

A complete example, which pulls this repo's pull requests and a lens that
matches them to tasks, is in
[`examples/extensions/github`](https://github.com/nvoxland/oxplow/tree/main/examples/extensions/github).
