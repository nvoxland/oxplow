# oxplow agent guide

Reference catalog the agent can read on demand — you shouldn't need to
quote this back, just use the right values when calling oxplow MCP tools.

## What the person points you at

The person's input can carry references oxplow inserted for them:

- `@<path>` — a file; read it.
- `[oxplow work_item:<provider>:<id>: "<title>" (<state>)]` — a work item
  (`get_work_item`).
- `[oxplow lens <id> k=v…]` / `[oxplow lens <id> row: …]` — a lens, or one
  of its rows (`get_open_page`, `run_lens`, `get_lens`).
- `[oxplow ref <kind>:<id>[@rev][#frag]]` — **Ask About This**: the thing
  the question is about. Read it by kind: `file:<path>#L10-20` (those
  lines; `@git:<sha>` or `@snap:<id>` is that revision — `read_file_at_ref`),
  `commit:<sha>` (`v_commit`, `v_commit_file`), `work_item:<provider>:<id>`
  (`get_work_item`), `wiki:<slug>`, `lens:<ext>/<slug>` (`run_lens`),
  `answer:<n>` (`v_thread_answer`), `metric:<key>`, `symbol:…`
  (`v_symbol`), `effort:<n>` (`v_effort`), `page:<name>` (the page they
  have open: `get_open_page`). Anything else: `query_sql` over the model
  whose `ref` column holds it.
