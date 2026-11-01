---
name: oxplow-extension
description: Build oxplow pages and extensions on request: views over oxplow's data (tasks, efforts, threads, comments, wiki, snapshots, metrics) that the user sees in the app and can share with their team. Loads when the user asks to "make/build/show me a page/view/report/dashboard/list of X", "a place where I can see X", "track X across threads/efforts", "what's waiting on me", or asks to change an existing page. Teaches the semantic layer (`v_dataset`, `oxplow_query_sql`), the `.oxplow/extensions/<name>/` format, and the fixture, check, test, deploy loop.
---

# Building oxplow pages

A **page** (`pages/<slug>.yaml`) is named queries (`loaders:`) plus a tree of
nodes that shows them (`view:`); it updates as the work changes. A **part**
(`parts/`) is drawn only inside a host (an embed, a mount, a panel) and has
no tab; a **query** (`queries/`) is rows only, for `options_from` and form
`choices`. Pages live in **extensions**: plain-file folders at
`.oxplow/extensions/<name>/` the user can keep and share. Scaffold with
`$OXPLOW_BIN extension new <kind> <name> --origin <your effort ref>`;
`extension add <kind> <id>` adds one more to an extension that exists.
Nothing runs a source until `oxplow.extension.deploy { name }`.

`$OXPLOW_BIN` is the oxplow command line (set in sessions oxplow starts;
elsewhere `oxplow`, once installed from Settings).

**Before building, look:** `oxplow_find_page { question }` finds the page
that already answers it; `oxplow_show_page { page }` shows it. **To answer
with a table or chart now**, `oxplow_show_page { spec }` (inline,
read-only) beats pasting rows; build a file only for a lasting page.

**The loop:** find the data (`v_dataset`, `v_field`, `oxplow_query_sql`:
read columns, never guess) → **write a fixture for the behavior first**
(`fixtures/<example>.yaml`: `given` the data, `when` it runs, `expect`
what it shows or does — a component's with `ui:` steps) → write the page
(or command, effect, component) → `oxplow_validate_extension` after every
edit → `$OXPLOW_BIN extension test <name>` until it passes (seconds; it
runs on a throwaway oxplow with only the data your fixtures make, never
the project's) → `oxplow.extension.deploy { name }` from the main stream
(it replaces what runs, live; never ask for a restart) → tell the user
the title and how to open it (Cmd+P). `oxplow_run_page { draft: true }`
shows a draft page over the project's real data: a look, not a test.

**Open the reference for the task** (a file here, or `oxplow_get_skill
{ name: oxplow-extension, reference: <name> }`):

- `references/page-format.md`: manifest, metadata, params, answers. Start here.
- `references/loaders.md`: `loaders:`: sql, command, page, rows.
- `references/view-nodes.md`: `view:`: layout, viz, columns, actions, embed.
- `references/recipes.md`: data to page, a form, a dashboard, a slot mount.
- `references/dashboards.md`: pages of embeds; Pin.
- `references/markdown-blocks.md`: `oxplow-page` blocks, `show_page`.
- `references/testing.md`: fixtures (`given` → `when` → `expect`, `steps`, `ui`), check, answers.
- `references/development-cycle.md`: source, drafts, deploy, held ones.
- `references/ui-contributions.md`: slots, panels, renderers, decorators, replacements.
- `references/scaffolding.md`: the kinds `extension new` makes.
- `references/semantic-layer.md`: finding data, URIs in SQL, datasets.
- `references/commands-and-forms.md`: commands, forms, components.
- `references/collectors.md`, `references/metrics.md`, `references/jobs-and-ai.md`, `references/providers.md`: outside data, metrics, model jobs, providers.
- `references/repair-and-sharing.md`: repair, share.
- `references/reading-data.md`: before reporting numbers.

Rules in other skills: wikilinks and URIs in prose, `oxplow-wiki-capture`;
waiting on the person, `oxplow-runtime`; tests in the foreground,
`oxplow-collection`; metrics, `oxplow-metrics`. A person approves an
extension's `permissions:` and a new `build:` command at deploy; your
deploy that adds one becomes a proposal.
