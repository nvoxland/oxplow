# effort-review

When an agent's effort finishes, a model reviews it from what oxplow
collected about it, and the review lands as a comment on the effort's
work item:

- the tests the agent ran (`v_test_run`) and the latest observation of
  each kind (`v_effort_observation`: the test run, diff coverage with the
  lines it leaves uncovered, static analysis);
- the files it changed and the functions in them, with their complexity
  change (`v_effort_file`, `v_change_file`, `v_change_function`);
- the metrics that moved (`v_effort_metric_delta`) and code-quality
  findings on its files (`v_code_quality_finding`).

The review streams into oxplow's **Jobs** page while the model writes,
and you can cancel it there (or with **Cancel** on **Effort Reviews**); a
cancelled review is still commented, marked as cancelled.

## What it contributes

| | |
|---|---|
| `jobs: review` | One model conversation per effort (`ai.complete_typed`), keyed `review:<effort id>`, so a second request for the same effort is the same job. Its one tool is `effort.evidence`; it may also query oxplow's models (`query_sql`). At most 3 tool rounds, 180 seconds. |
| `effort.evidence` | The evidence above in one answer, every list capped (50 files, 50 functions, 50 findings, 20 test runs and metrics) with `truncated` naming what was cut. A person or an agent can run it too. |
| `effort.review` | Asks for one effort's review: the one place the prompt lives. |
| `review.ask` / `review.status` / `review.cancel` | The job's lifecycle, through oxplow's jobs service. |
| `review-on-finish` | On `effort.finished` (not `effort.closed`: the evidence is refreshed between the two), asks for the review of an effort with a work item. |
| `publish-review` | On `contribution.job.completed`, reads the review back and comments it on the effort's work item. |
| `reviews` dataset | `v_effort_review_reviews`: each review, its effort and work item, its state and what the dataset wrote. **Exported** as `oxplow:oxplow/dataset?dataset=effort-review/reviews` v2, so another extension can build on it. |
| Pages | **Effort Reviews** and **Finished Efforts** (with **Review**), both under Work. |

## What it needs

| Grant | Who | Why |
|---|---|---|
| `ai.compute`, `sql.read` | the `review` job | the model, and its `query_sql` tool |
| `sql.read` | `effort.evidence` | reading the evidence |
| `jobs.read`, `sql.read` | `publish-review` | reading the review back, and the effort's work item |

It runs no program, reads no files, opens no network connection and holds
no credentials: the model is whichever one you bind to the AI profile
**effort-review/review** in Settings → AI, or your **Summarize** model when
you bind none, through your own AI provider. Sharing the folder shares nothing of
yours.

## Use it

1. Copy this folder to `.oxplow/extensions/effort-review/`.
2. Settings → AI: bind **effort-review/review** to a model (it needs tool
   calling), or give the **Summarize** role one and it is used instead.
3. Deploy it (`oxplow.extension.deploy { name: "effort-review" }`, or
   Deploy on Settings → Extensions) and approve what it asks for: models
   (`ai`), the agent's text and its own commands. Until it is deployed
   no review starts and nothing is sent to a model.
4. Finish an effort that has a work item, or press **Review** on
   **Finished Efforts**.

Changing `extension.yaml` takes a redeploy; it asks again only if it adds a permission.

## Build on it

Another extension reads the reviews through the exported dataset:

```yaml
imports:
  - { id: reviews, contribution: oxplow:oxplow/dataset?dataset=effort-review/reviews, version: 2, optional: true }
```

and `SELECT … FROM import_ref('reviews')` in its pages or datasets. With
`optional: true`, what reads it is withheld (with a warning in Settings →
Extensions) when effort-review isn't installed, is disabled or exports
another version; `oxplow extension check` prints how the import resolved.

## Test it

`oxplow extension test .oxplow/extensions/effort-review` runs its eight
examples on a throwaway oxplow with a fixture model: the job with a tool
call over an effort its `setup` makes (a work item, the agent's test run),
answered once the evidence carries the test run, cancelled mid-reply, its end reaching `publish-review`, the effect
on a finished effort (with and without a work item), the evidence bundle
from given rows, and the Reviews page.
