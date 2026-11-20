## Working in oxplow

- oxplow records your turns from what it observes: there's nothing to
  report about them. Whether you track your work as tasks is the task
  tracker's business: when one is active, its skill says how, and you
  follow it. For the command bus, read the `oxplow-runtime` skill.
- The project keeps knowledge pages — durable understanding, in one or
  more stores (oxwiki is oxplow's own). Find them with `v_oxplow_knowledge_page`
  or `oxplow_search`, read one with `oxplow_read`, keep one with
  its store's command (oxwiki: `oxwiki.page.write`; the `knowledge` and `oxwiki` skills).
- When your reply needs the person's answer, end it with the question
  itself: oxplow shows your thread as waiting on them.
- When the person mentions an error they saw in the app (a red toast, a row
  on the Alerts page), you can read it: `v_op_error` through `oxplow_query_sql`, its
  output in `v_op_error_output` (the `oxplow-codebase` skill).
- The person's message may carry references oxplow inserted: `@<path>`
  (a file), `[oxplow uri <uri>]` (what they're asking about, a URI such as
  `oxplow:oxplow/commit?sha=abc1234`),
  `[oxplow <task uri>: "<title>" (<state>)]`, `[oxplow page <id> …]`. Read a URI
  with `oxplow_read` (a file at a revision: `oxplow:oxplow/file?path=<path>&rev=git:<sha>#lines=10-20`;
  anything a dataset's `uri` column holds); the page they have open with
  `oxplow_get_open_page`.
