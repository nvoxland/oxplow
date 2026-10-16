---
name: oxplow-extension
description: Build oxplow lenses and extensions on request: custom pages over oxplow's data (tasks, efforts, threads, comments, wiki, snapshots, metrics facts) that the user sees in the app and can share with their team. Loads when the user asks to "make/build/show me a lens/page/view/list/report of X", "a place where I can see X", "track X across threads/efforts", "what's waiting on me", or asks to change an existing lens. Teaches the semantic layer (`v_model`, `oxplow_query_sql` over `v_*` views), the `oxplow/extensions/<name>/` file format, and the validate → oxplow_run_lens loop.
---


# Building oxplow lenses

A **lens** is a saved query over oxplow's data plus how to show it, opened
as a page; it updates as the work changes. Lenses live in **extensions**:
plain-file folders at `oxplow/extensions/<name>/` (`extension.yaml`,
`lenses/<slug>.yaml`) the user can keep, commit for their team or publish.
Scaffold with `oxplow extension new <kind> <name> --origin <your effort
ref>`, then edit; the scaffold passes `check` as written.

**To answer a question** with a table, chart or list, call `oxplow_show_lens`
(an existing `lens` plus `params`, or your own `spec`) instead of pasting
rows; build a lens file only for a lasting page.

**The loop:** find the data (`v_model`, `v_model_column`, `oxplow_query_sql`:
read the columns, never guess) → write the files → `oxplow_validate_extension`
(`oxplow extension check`) after every edit → `oxplow_run_lens` → tell the user
the title and how to open it (Cmd+P). Pass your own `stream_id` when in a
worktree stream. `check` always dry-runs, so no sync is needed first.

**Open the reference for the task at hand** (a file here, or `oxplow_get_skill`
with `name: oxplow-extension` and `reference: <name>`):

- `references/scaffolding.md`: the kinds `extension new` makes, the words used exactly.
- `references/semantic-layer.md`: finding data, refs in SQL and scripts, publishing and
  importing models.
- `references/lens-format.md`: manifest, lens keys, viz, links, row styling, alerts, actions.
- `references/ui-contributions.md`: slots, pages, panels, decorators, replacing a core view.
- `references/commands-and-forms.md`: your own commands, custom components, forms.
- `references/collectors.md`: outside or derived data, credentials, host services, `ai_*`.
- `references/metrics.md`: measures, metrics, dimensions, fact collectors.
- `references/jobs-and-ai.md`: model jobs with tools, `ai_profiles`.
- `references/check.md`: what `check` covers, `oxplow_run_lens`, the extension's `questions.yaml`.
- `references/testing.md`: `oxplow_preview_extension` and the fixture shape of every kind.
- `references/providers.md`: external providers: testing, credentials, MCP adapters.
- `references/lenses-in-use.md`: `oxplow_show_lens` details, changing "this lens", handing over.
- `references/repair-and-sharing.md`: disabled contributions and repair, install, share.
- `references/reading-data.md`: before you report numbers: `truncated`, `coverage`, metric folds, scope, example queries.
- `references/dashboards.md`: choosing a lens, dashboard, component or snapshot; building a dashboard; adapting outside HTML into a component.

Rules that live in other skills: wikilinks and refs in prose, see
`oxplow-wiki-capture`; waiting on the person, `oxplow-runtime`; test
commands run in the foreground, `oxplow-collection`; metrics, `oxplow-metrics`.
A person approves programs, collectors and installs; you can't.
