# review-badges

The latest model review of each work item, from
[effort-review](../effort-review), as a list and on the task's own page.
It reads effort-review's exported `reviews` dataset and writes nothing.

| | |
|---|---|
| **Review Verdicts** (Work) | One row per work item: its latest review's effort, how it ended, when, and the first 120 characters. The work item links to its page. |
| **Review** (the task page, among its sections) | That work item's latest review in full, if there is one. Mounted on the `oxplow-bundled/work-item.sections` slot, which passes the item's `uri`. |

```yaml
imports:
  - { id: reviews, contribution: oxplow:oxplow/dataset?dataset=effort-review/reviews, version: 2, optional: true }
ui:
  mounts:
    - { page: item-review, at: oxplow-bundled/work-item.sections }
```

Both pages read `import_ref('reviews')`, which is how an extension reads
another's exported dataset: only what effort-review published, by the
version it declared.

## It needs effort-review

Install both: copy `effort-review/` and `review-badges/` into
`.oxplow/extensions/`, then follow effort-review's README (a dataset on the
**Summarize** role, deployed). Reviews appear as efforts finish.
review-badges itself runs no program and declares no permissions.

## Without effort-review

The import is `optional: true`, so a missing producer is not an error.
When effort-review is missing, disabled, or exports another version:

- the two pages and the slot are **withheld**: the list isn't in the
  launcher and nothing is mounted on the task page;
- **Settings → Extensions** shows review-badges with a warning that says
  which import is unavailable and why;
- everything else, including review-badges itself, loads as usual.

`oxplow extension check review-badges` prints how the import resolved:
``import `reviews` → oxplow:oxplow/dataset?dataset=effort-review/reviews v2: available`` or
`unavailable (effort-review is disabled)`, `(effort-review exports v2)`,
`(effort-review or its contribution isn't here)`. `oxplow extension test`
prints the same, and skips the fixtures for the two pages with a warning
saying the import withheld them and why.

## Test it

`oxplow extension test .oxplow/extensions/review-badges` runs the two page
examples (both empty in a fresh project).
`oxplow-sdk/tests/examples.rs` also runs it beside effort-review: present,
on another version and missing.
