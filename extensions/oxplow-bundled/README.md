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

## What it reads

- `v_claim` — what the agent said about its work. `verified = 0` means
  nothing backs it: no `evidence_ref`, and for a `tests_pass` claim no
  passing test run in the effort.
- `v_decision` — the forks the agent resolved. `provenance = 'inferred'`
  rows are oxplow's guesses from the conversation, until a person
  confirms or dismisses them.
- `v_oxplow_bundled_deviation` (this extension's model) — files an effort
  changed outside the area its work item names. A file is in the area
  when the item's title or body names it, or one of its directories at
  least two levels deep (`src/ui`). An item that names no area has none.
- `v_effort_file`, `v_work_item` — what the packet's other lenses show.

## What a reviewer does

On an effort's page, **Commands**:

- **Accept Review** (`oxplow.review.accept { ref, force? }`) comments the
  review on the effort's work item and marks it done. It refuses while a
  claim is unverified or an inferred decision unreviewed; `force` accepts
  anyway and lists them in the comment.
- **Request Changes** (`oxplow.review.request_changes { ref, note? }`)
  comments a checklist — each unverified claim, each inferred decision,
  each file outside the area, and the note — and moves the item back to
  todo (an oxplow task: ready).

Each logs its verdict as an event (`event_types:` in the manifest):
`oxplow_bundled.accepted { unverified, inferred, deviated }` or
`oxplow_bundled.changes_requested { unverified, inferred, deviated, note? }`,
caused by the command's run. The counts are what stood at the verdict —
claims unverified, inferred decisions unreviewed, files outside the area.
The verdict itself is in the envelope, which is kept when the payload
expires (30 days): the type says which, and the subject names the effort,
its work item and — for an acceptance — every claim and decision accepted
unchecked (any at all makes it forced). Read them in `v_event`, or react
to one from another extension's effect (`on: [oxplow_bundled.accepted]`).

Both work on any provider's item. On an oxplow task the comment and the
move are one change you can undo; on another provider's item
they run in order through the provider — if the move fails the comment
stays — and can't be undone from oxplow. An effort with no work item is
refused.

On the packet's rows: **Mark Verified** on an unverified claim
(`oxplow.effort.verify_claim`), **Confirm** / **Dismiss** on an inferred
decision (`oxplow.effort.confirm_decision`, `oxplow.effort.dismiss_decision`), and the
Verify a Claim With Evidence form for citing a test run or a file.

All of these are a person's: an agent can read the packet, but it can't
accept or reject its own work, verify its own claims or review its own
decisions. Ask the person to.
