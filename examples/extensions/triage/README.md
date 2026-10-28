# Triage: TODOs sorted by a model

Every TODO comment in your code, classified as a bug, cleanup or a
feature, next to the number of open work items. It's one fact collector
(`collectors/todos.star`) that uses all three host services:

- **`tree.read`**: `source_files()` and `markers()` find the TODOs in the
  snapshot. A fact collector reads its tree without declaring it.
- **`sql.read`**: `scope("sql.read", …)` counts the open work items.
- **`ai.compute`**: `ai_classify()` sorts each TODO. It spends your AI
  provider's key, so the extension declares `permissions.ai` and the
  collector runs once it is deployed with that approved. Each
  answer is recorded, so an unchanged TODO isn't asked about again.

```yaml
collectors:
  - id: todos
    runtime: starlark
    entry: collectors/todos.star
    on: [snapshot.taken]
    needs: [sql.read, ai.compute]
    facts: [triage.todos, triage.open_items]
```

`oxplow extension check triage --services` lists what the script uses
and whether it declared it:

```
collector `todos` (collectors/todos.star):
  sql.read: declared, used
  tree.read: implied, used
  ai.compute: declared, used — under permissions.ai
```

## A summary job, and a board you decide on

- **A job** (`jobs: summarize`): `triage.summary.ask` has a model summarize
  the TODOs outside any transaction. Its reply streams into `v_job_run_output`
  as it comes; its model may call `triage.todos.in_file` (a read-only
  command that admits a job) or `query_sql` before answering; when it
  ends, the `publish-summary` effect writes the answer to the wiki.
  `triage.summary.cancel` stops it, keeping what it had written.
- **Decisions in private state.** Each TODO can be marked keep, fix now
  or won't fix. The decisions live in the extension's own state
  (`state.write`), never in SQL: `triage.todos.decide` reads the state
  and composes `triage.state.set` against the stamp it read, so two
  people deciding at once can't lose one's call; `triage.todos.decisions`
  reads them back beside the TODOs.
- **The TODO board** (`components/todo-board`) shows the TODOs by kind,
  each card with a form to decide on it. It runs the two commands above,
  so it works once the extension is deployed; if they are refused (not
  deployed, or disabled) it shows the TODOs without decisions and says so. The `todos` page (under Code in
  the launcher) has the same decision as a row action, `Decide…`, which
  opens a form.

**What survives.** Decisions are in the extension's private state, in
oxplow's database, so they outlive what a person does around them: a
restart finds them; disabling the extension stops the board deciding or
reading them until it is enabled again, and enabling brings them back;
editing the bundle takes a redeploy (the deploy asks again only when it
adds a permission), and the decisions read back once it is live.
Removing the extension keeps the rows, but nothing reads them. Two decisions in a row each read the stamp
the last left; a write holding an old stamp is refused rather than
overwriting. `oxplow-sdk/tests/trials.rs` runs each of these over an
on-disk database.

The commands read the TODOs as the board's page does, through the
metric (`metric_findings('triage.todos')`).

## Use it

1. Copy this folder to `.oxplow/extensions/triage/` in your repo.
2. Settings → AI: bind **triage/sort** to a model, or give the **Decide**
   role one and it is used instead.
3. Deploy it (`oxplow.extension.deploy { name: "triage" }`, or Deploy on
   Settings → Extensions) and approve what it asks for. A later deploy
   asks again only if it adds a permission.
4. The next snapshot records `triage.todos`, one fact per TODO with its
   kind in `rule`, and `triage.open_items`.

## What it can't do

A command's, an effect's or an effort policy's script composes commands
and may run more than once, so it never asks a model. Move the
`ai_classify` call into one of those and the extension doesn't load:
`check` names the call and why.
