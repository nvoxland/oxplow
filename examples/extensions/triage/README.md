# Triage: TODOs sorted by a model

Every TODO comment in your code, classified as a bug, cleanup or a
feature, next to the number of open work items. It's one fact collector
(`collectors/todos.star`) that uses all three host services:

- **`tree.read`**: `source_files()` and `markers()` find the TODOs in the
  snapshot. A fact collector reads its tree without declaring it.
- **`sql.read`**: `scope("sql.read", …)` counts the open work items.
- **`ai.compute`**: `ai_classify()` sorts each TODO. It spends your AI
  provider's key, so the collector runs only once you approve it. Each
  answer is recorded, so an unchanged TODO isn't asked about again.

```yaml
collectors:
  - id: todos
    runtime: starlark
    entry: collectors/todos.star
    trigger: { on: [snapshot.taken] }
    needs: [sql.read, ai.compute]
    facts: [triage.todos, triage.open_items]
```

`oxplow extension check triage --services` lists what the script uses
and whether it declared it:

```
collector `todos` (collectors/todos.star):
  sql.read: declared, used
  tree.read: implied, used
  ai.compute: declared, used — a person approves it
```

## Use it

1. Copy this folder to `oxplow/extensions/triage/` in your repo.
2. Settings → AI: give the **Decide** role a model.
3. Settings → Data: approve **triage/todos**. Any change to its files
   asks again.
4. The next snapshot records `triage.todos`, one fact per TODO with its
   kind in `rule`, and `triage.open_items`.

## What it can't do

A command's, an effect's or an effort policy's script composes commands
and may run more than once, so it never asks a model. Move the
`ai_classify` call into one of those and the extension doesn't load:
`check` names the call and why.
