# One effort per prompt: an example effort policy

oxplow's own effort policy opens an effort when a turn first changes files
and keeps it open across prompts until a commit lands its changes or its
work item finishes. This one gives **every prompt that changes files an
effort of its own**, so an effort's review packet, files, test runs and
tokens are exactly that prompt's.

It's a script (`policies/prompt_efforts.star`): an effort policy hears
core's events and answers with the commands to run.

- **A turn that changed the worktree** (`thread.checkpoint` with `changed`
  and a writing tool): the thread's open effort closes as of the turn's
  start, and a new one opens adopting the turn, carrying the previous
  effort's work item. A question-only turn gets nothing; a turn whose
  effort was already opened during it (an item started mid-turn) is left
  alone.
- **A work item started** on a thread links the thread's open effort to it,
  or opens one. **A work item finished** closes its effort. Every effort
  policy keeps this floor; `oxplow extension test` checks it.

## Use it

1. Copy this folder to `oxplow/extensions/prompt-efforts/` in your repo.
2. Settings → Data → Programs: approve **Effort policy
   prompt-efforts/prompt_efforts** (it reads your efforts and turns
   through `sql.read`; any change to its files asks again).
3. Settings → Capabilities: choose **One effort per prompt** as the effort
   policy. Switching closes every open effort; the next prompt that changes
   files opens the first one.

## Files

- `extension.yaml`: the `effort_policy` implementation and its examples.
- `policies/prompt_efforts.star`: the policy.
- `fixtures/`: each example's event, the reads it answers, and what the
  policy composes; `oxplow extension test` dry-runs them and runs the
  effort-policy suite over the script.
