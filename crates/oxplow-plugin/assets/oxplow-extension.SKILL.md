---
name: oxplow-extension
description: Build oxplow lenses and extensions on request: custom pages over oxplow's data (tasks, efforts, threads, comments, wiki, snapshots, metrics facts) that the user sees in the app and can share with their team. Loads when the user asks to "make/build/show me a lens/page/view/list/report of X", "a place where I can see X", "track X across threads/efforts", "what's waiting on me", or asks to change an existing lens. Teaches the semantic layer (`describe_schema`, `query_sql` over `v_*` views), the `oxplow/extensions/<name>/` file format, and the validate → run_lens loop.
---

# Building oxplow lenses

A **lens** is a saved query over oxplow's data plus how to show it. The user
opens it as a page (Cmd+P → Lenses), and it updates as the work changes.
Lenses live in **extensions**: folders of plain files in the repo. The user
can use a lens themselves, share it with their team by committing it, or
publish the extension for anyone.

You build them with your normal file tools. No special tool writes them.

## 1. Find the data

- `describe_schema` lists every queryable view (`v_stream`, `v_thread`,
  `v_task`, `v_effort`, `v_comment`, `v_wiki_page`, `v_snapshot`,
  `v_measure`, `v_capture`, `v_fact`, and anything extensions add), with a
  doc for every column. **Read it first**; don't guess column names.
- `query_sql` runs one read-only `SELECT`/`WITH`. Iterate on the query
  here until the rows are what the user wants, before writing any files.
- Only query `v_*` views. The physical tables behind them aren't a stable
  contract.

## 2. Write the files

In the worktree of the stream you're working in:

```
oxplow/extensions/<name>/extension.yaml
oxplow/extensions/<name>/lenses/<slug>.yaml
```

`<name>` and `<slug>` are lowercase-with-dashes. The lens id is
`<name>/<slug>`. Reuse an existing extension when the lens fits it; start a
new one for a new area.

`extension.yaml`:

```yaml
name: review            # must equal the folder name
description: Lenses for reviewing agent work
```

`lenses/<slug>.yaml`:

```yaml
title: Waiting on me
description: Blocked tasks and open follow-up comments in this stream.
params:                         # optional; bound as :name in the query
  - { name: stream, label: Stream, default: 1 }
query: |
  SELECT id, title, status, thread_id
  FROM v_task
  WHERE status = 'blocked' AND stream_id = :stream
  ORDER BY updated_at DESC
viz: table                      # table | list | number | markdown
columns:                        # optional; controls headers, order and links
  - { key: title, label: Task, link: { kind: task, from: id } }
  - { key: status }
empty: Nothing is waiting on you.
```

- **`viz`**:
  - `table` shows rows.
  - `list` shows one line per row, headlined by the first column.
  - `number` shows the first cell as a big number.
  - `markdown` renders the first cell as markdown.
- **`link.kind`** makes cells clickable:
  - `task` takes a task id.
  - `file` takes a repo-relative path.
  - `wiki` takes a slug.
  - `effort-diff` takes an effort id.
  
  `from` names the result column that holds the id; it defaults to the
  column itself.
- **Unknown keys are errors.** Only the keys shown above exist today.

## 3. Check it

1. `validate_extension(name, stream_id)` reports:
   - load errors (YAML, unknown keys, a name/folder mismatch);
   - a dry run of every lens with its default params (SQL errors, and
     `columns` keys the query doesn't return).

   Fix everything it reports.
2. `run_lens(id, params?, stream_id)` returns exactly the rows the user
   will see. Check that they answer the question.
3. Pass **your own `stream_id`** to both when you're in a worktree
   stream. Extensions are read from the stream's worktree, so the primary
   stream won't see yours until it's merged.

## 4. Hand it over

Tell the user the lens title and how to open it (Cmd+P, type the title).
Offer to commit it if they want their team to have it. Lens files are
ordinary project files, so they go through the normal task/effort flow:
file or pick up a task before editing, and list the files in
`touched_files` when you close it.

## Working with the user's lenses

- `list_lenses` / `get_lens` show what already exists. Prefer improving an
  existing lens over adding a near-duplicate.
- When the user says "this lens" or pastes `[oxplow lens <id>]`, read it
  with `get_lens`, change the file, then validate and run it again.
