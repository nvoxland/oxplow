# Answering with a lens, handing it over, changing the user's lenses

## Answering with a lens (`oxplow_show_lens`)

When the user asks a question whose answer is a table, chart or list —
"which files changed most this week?" — **show** it rather than pasting
rows into the terminal: call `oxplow_show_lens` with either `lens` (an existing
lens id, plus `params`) or `spec` (a lens of your own: `title`, `query`,
`viz` and what the viz needs, the same keys as a lens file). oxplow runs
the query read-only (as `oxplow_query_sql` does), stores the answer on the
thread and shows it beside the conversation; the tool returns the
answer's ref and its text rendering, which is what you tell the user.
An answer is never a `form` (its submit would run a command as the
user), a `grid` or a `custom` component.

Nothing is written to the repo. The user presses **Keep This** to turn an
answer into a private lens (in `my-lenses`), and shares it from there. Build
a lens file (`references/lens-format.md`) only when they ask for a lasting page.

## Hand it over

Tell the user the lens title and how to open it (Cmd+P, type the title).
Offer to commit it if they want their team to have it. Lens files are
ordinary project files, so they go through the normal task/effort flow
(the files you write are observed).

## Working with the user's lenses

- `v_lens` (`uri`, `extension`, `title`, `description`, `viz`, `hidden`) lists
  what already exists, `v_extension` the extensions that loaded (`enabled`,
  `errors`, `warnings`) and `v_contribution` what each contributes (count it
  by `extension` and `kind`); `oxplow_read { uri: "oxplow:oxplow/lens?lens=<ext>/<slug>" }` gives one
  lens's definition. Prefer improving an existing lens over adding a
  near-duplicate. An extension that fails to load isn't in `v_extension`:
  `oxplow_validate_extension` says why.
- When the user says "this lens", "what I'm looking at" or pastes
  `[oxplow lens <id>]`:
  1. Call `oxplow_get_open_page(thread_id)`. Its `page` is what they look at (the URI
     without its `#`), `view` their filters and picks, and `lens` is what is on their
     screen as text, with their current params (`oxplow_run_lens` with
     `format: "json"` gives you the raw rows if you need them).
  2. Read the definition with `oxplow_read { uri: "oxplow:oxplow/lens?lens=<ext>/<slug>" }`.
  3. Change the file.
  4. Validate and run it again.
