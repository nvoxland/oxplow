# oxplow-bundled

What comes with oxplow, built as an ordinary extension:

- the effort review packet: what a person checks before accepting an
  agent's work, shown on each effort's diff (the `effort.review.details`
  slot), and the verdicts they give;
- the analytics lenses: tests and coverage, metric deltas, churn,
  duplication, co-change, token use and page visits;
- advisories that steer the agent toward tests and metric thresholds
  while an effort is open.

The rest of this page is about the review packet.

## Settings

The review policy's constants are this extension's `configuration:`, with
today's values as the defaults, so a project tunes them without forking:

| Key | Default | Reads |
|---|---|---|
| `changeInterest.complexityWeight`, `.parameterWeight` | 0.6, 0.4 | "Look here first" weights for complexity added and parameters added |
| `changeInterest.longFunctionLines`, `.longFunctionScale` | 60, 40 | a new function longer than this raises the file's priority; lines over it that double the factor |
| `coChange.minShared`, `.windowDays`, `.maxFilesPerCommit` | 3, 180, 50 | what makes two files a co-change pair |
| `coChange.dormantDays` | 90 | a file no commit touched in this long is dormant |
| `struggle.repeatedEdits`, `.failedCommands` | 5, 3 | when edits of one file, or failed shell commands, count as struggle |
| `largeUncommitted.files` | 15 | files an open effort holds before the agent is told to commit |
| `sharedFiles.windowMinutes` | 60 | how recent an edit is for two threads' open efforts changing the same file to count as overlapping work |

`oxplow.review_policy.read` returns the effective values and where each came
from; `oxplow.review_policy.write` sets the project's or the person's layer
(a compare-and-swap on the stamps `read` returns). A change takes effect
without a restart: the models that read it are republished (and the
materialized ones refilled), and the advisory reads it on its next run.

## What it reads

- `v_claim` — what the agent's answers said about its work, as a model
  read them when the effort finished (nothing the agent records), what
  each cites, and a reviewer's `review` (`verified`, `dismissed`, or NULL).
  `v_oxplow_bundled_claim_status` (this extension's model) says whether
  something backs it: `verified = 0` means no `evidence_ref`, no person's
  verification, and for a `tests_pass` claim no passing latest test run in
  the effort. A dismissed claim has no row there: it counts for nothing.
- `v_decision` — the forks the agent resolved. `provenance = 'inferred'`
  rows are a model's guesses from the conversation, until a person
  confirms or dismisses them (below, "Decisions Oxplow Noticed").
- `v_oxplow_bundled_deviation` (this extension's model) — files an effort
  changed outside the area its work item names. A file is in the area
  when the item's title or body names it, or one of its directories at
  least two levels deep (`src/ui`). An item that names no area has none.
- `v_effort_file`, `v_work_item` — what the packet's other lenses show.

## What a reviewer does

On an effort's page, **Commands**:

- **Accept Review** (`oxplow.review.accept { effort, force? }`) comments the
  review on the effort's work item and marks it done. It refuses while a
  claim is unverified (a dismissed one doesn't count) or an inferred
  decision unreviewed; `force` accepts
  anyway and lists them in the comment.
- **Request Changes** (`oxplow.review.request_changes { effort, note? }`)
  comments a checklist — each unverified claim, each inferred decision,
  each file outside the area, and the note — and moves the item back to
  todo.

Each logs its verdict as an event (`event_types:` in the manifest):
`oxplow_bundled.accepted { effort, work_item, unverified, inferred, deviated }` or
`oxplow_bundled.changes_requested { effort, work_item, unverified, inferred, deviated, note? }`,
caused by the command's run. The counts are what stood at the verdict —
claims unverified, inferred decisions unreviewed, files outside the area.
The verdict itself is in the envelope, which is kept when the payload
expires (30 days): the type says which, and the subject names the effort,
its work item and — for an acceptance — every claim and decision accepted
unchecked (any at all makes it forced). Read them in `v_event`, or react
to one from another extension's effect (`on: [oxplow_bundled.accepted]`).

Both work on any list's item. On oxplow's own tasks the comment and the
move are one change you can undo; on another provider's item
they run in order through the provider — if the move fails the comment
stays — and can't be undone from oxplow. An effort with no work item is
refused.

On the packet's rows: **Mark Verified** on an unverified claim
(`oxplow.effort.verify_claim`), **Dismiss** for a claim the agent didn't
make or didn't mean (`oxplow.effort.dismiss_claim`; it stops counting) and
**Reopen** to take that back (`oxplow.effort.reopen_claim`), **Confirm** / **Dismiss** on an inferred
decision (`oxplow.effort.confirm_decision`, `oxplow.effort.dismiss_decision`), and the
Verify a Claim With Evidence form for citing a test run or a file.

All of these are a person's: an agent can read the packet, but it can't
accept or reject its own work, verify its own claims or review its own
decisions. Ask the person to.

## Decisions Oxplow Noticed

When an effort finishes, the `infer-reasoning` effect asks a model which
decisions the agent made without recording them and which claims its
answers made about its work. `effort.infer_reasoning` digests the effort
— its conversation (each turn's prompt and final answer), tool calls and
the decisions it did record — into one prompt for the `reasoning` job,
and the `store-reasoning` effect stores the reply as the effort's
inferred decisions and claims through oxplow's
`oxplow.effort.replace_inferred_decisions` and
`oxplow.effort.replace_inferred_claims` (the latter replaces only claims
no reviewer has acted on).

- **The model** is the person's binding of `oxplow-bundled/reasoning` in
  Settings → AI, else their Main model. With neither, nothing is asked:
  the effect skips and says where to bind one.
- **Approve** the job `reasoning` and the effects `infer-reasoning` and
  `store-reasoning` (Settings → Data → Programs) before anything runs.
- **Infer Again** on the list (`effort.infer_reasoning { again: true }`)
  asks once more, e.g. after binding another model; an unchanged prompt
  on the same model reads the answer it already has.
- A reply that isn't the JSON asked for is skipped, saying so.
