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
| `jobs: review` | One model conversation per effort (`ai.complete_typed`), keyed `review:<effort ref>`, so a second request for the same effort is the same job. Its one tool is `effort.evidence`; it may also query oxplow's models (`query_sql`). At most 3 tool rounds, 180 seconds. |
| `effort.evidence` | The evidence above in one answer, every list capped (50 files, 50 functions, 50 findings, 20 test runs and metrics) with `truncated` naming what was cut. A person or an agent can run it too. |
| `effort.review` | Asks for one effort's review: the one place the prompt lives. |
| `review.ask` / `review.status` / `review.cancel` | The job's lifecycle, through oxplow's jobs service. |
| `review-on-finish` | On `effort.finished` (not `effort.closed`: the evidence is refreshed between the two), asks for the review of an effort with a work item. |
| `publish-review` | On `contribution.job.completed`, reads the review back and comments it on the effort's work item. |
| `reviews` model | `v_effort_review_reviews`: each review, its effort and work item, its state and what the model wrote. **Exported** as `effort-review/model/reviews` v1, so another extension can build on it. |
| Lenses | **Effort Reviews** and **Finished Efforts** (with **Review**), both under Work. |

## What it needs

| Grant | Who | Why |
|---|---|---|
| `ai.compute`, `sql.read` | the `review` job | the model, and its `query_sql` tool |
| `sql.read` | `effort.evidence` | reading the evidence |
| `jobs.read`, `sql.read` | `publish-review` | reading the review back, and the effort's work item |

It runs no program, reads no files, opens no network connection and holds
no credentials: the model is whichever one you put on the **Summarize**
role, through your own AI provider. Sharing the folder shares nothing of
yours.

## Use it

1. Copy this folder to `oxplow/extensions/effort-review/`.
2. Settings → AI: give the **Summarize** role a model.
3. Settings → Data → Programs: approve **Job effort-review/review** and
   the two effects. Until the job is approved, a review waits as
   `queued` on the Jobs page and nothing is sent to a model.
4. Finish an effort that has a work item, or press **Review** on
   **Finished Efforts**.

Changing `extension.yaml` asks for the job's approval again.

## Build on it

Another extension reads the reviews through the exported model:

```yaml
imports:
  - { id: reviews, contribution: effort-review/model/reviews, version: 1, optional: true }
```

and `SELECT … FROM import_ref('reviews')` in its lenses or models. With
`optional: true`, what reads it is withheld (with a warning in Settings →
Extensions) when effort-review isn't installed, is disabled or exports
another version; `oxplow extension check` prints how the import resolved.

## Test it

`oxplow extension test oxplow/extensions/effort-review` runs its eight
examples on a throwaway oxplow with a fixture model: the job with a tool
call, cancelled mid-reply, its end reaching `publish-review`, the effect
on a finished effort (with and without a work item), the evidence bundle
from given rows, and the Reviews lens.
