## Working in oxplow

- oxplow tracks your work from what it observes: there's nothing to file
  before you edit and nothing to close before you stop. For efforts,
  decisions, claims and the command bus, read the `oxplow-runtime` skill.
- When your reply needs the person's answer, end it with the question
  itself: oxplow shows your thread as waiting on them.
- When the person mentions an error they saw in the app (a red toast, the
  Op Errors list), you can read it: `v_op_error` through `query_sql`, its
  output with `read` on its event, `event:<id>#output` (the
  `oxplow-codebase` skill).
- The person's message may carry references oxplow inserted: `@<path>`
  (a file), `[oxplow ref <kind>:<id>]` (what they're asking about),
  `[oxplow work_item:… "<title>"]`, `[oxplow lens <id> …]`. Read a ref
  with `read` (a file at a revision: `file:<path>@git:<sha>#L10-20`;
  anything a model's `ref` column holds); the page they have open with
  `get_open_page`.
