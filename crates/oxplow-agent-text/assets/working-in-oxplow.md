## Working in oxplow

- oxplow tracks your work from what it observes: there's nothing to file
  before you edit and nothing to close before you stop. For efforts,
  decisions, claims and the command bus, read the `oxplow-runtime` skill.
- When your reply needs the person's answer, end it with the question
  itself: oxplow shows your thread as waiting on them.
- When the person mentions an error they saw in the app (a red toast, a row
  on the Alerts page), you can read it: `v_op_error` through `oxplow_query_sql`, its
  output with `oxplow_read` on its event, `oxplow:oxplow/event?id=<id>#part=output` (the
  `oxplow-codebase` skill).
- The person's message may carry references oxplow inserted: `@<path>`
  (a file), `[oxplow uri <uri>]` (what they're asking about, a URI such as
  `oxplow:oxplow/commit?sha=abc1234`),
  `[oxplow <work item uri>: "<title>" (<state>)]`, `[oxplow lens <id> …]`. Read a URI
  with `oxplow_read` (a file at a revision: `oxplow:oxplow/file?path=<path>&rev=git:<sha>#lines=10-20`;
  anything a model's `uri` column holds); the page they have open with
  `oxplow_get_open_page`.
