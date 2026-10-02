# oxplow-review

The effort review packet: what a person checks before accepting an
agent's work, shown on each effort's diff (the `effort.review.details`
slot), and the verdicts they give.

## What it reads

- `v_claim` — what the agent said about its work. `verified = 0` means
  nothing backs it: no `evidence_ref`, and for a `tests_pass` claim no
  passing test run in the effort.
- `v_decision` — the forks the agent resolved. `provenance = 'inferred'`
  rows are oxplow's guesses from the conversation, until a person
  confirms or dismisses them.
- `v_oxplow_review_deviation` (this extension's model) — files an effort
  changed outside the area its work item names. A file is in the area
  when the item's title or body names it, or one of its directories at
  least two levels deep (`src/ui`). An item that names no area has none.
- `v_effort_file`, `v_work_item` — what the packet's other lenses show.

## What a reviewer does

On an effort's page, **Commands**:

- **Accept Review** (`oxplow_review.accept { ref, force? }`) comments the
  review on the effort's work item and marks it done. It refuses while a
  claim is unverified or an inferred decision unreviewed; `force` accepts
  anyway and lists them in the comment.
- **Request Changes** (`oxplow_review.request_changes { ref, note? }`)
  comments a checklist — each unverified claim, each inferred decision,
  each file outside the area, and the note — and moves the item back to
  todo (an oxplow task: ready).

Both work on any provider's item. On an oxplow task the comment and the
move are one change you can undo; on another provider's item (Linear, …)
they run in order through the provider — if the move fails the comment
stays — and can't be undone from oxplow. An effort with no work item is
refused.

On the packet's rows: **Mark Verified** on an unverified claim
(`effort.verify_claim`), **Confirm** / **Dismiss** on an inferred
decision (`effort.confirm_decision`, `effort.dismiss_decision`), and the
Verify a Claim With Evidence form for citing a test run or a file.

All of these are a person's: an agent can read the packet, but it can't
accept or reject its own work, verify its own claims or review its own
decisions. Ask the person to.
