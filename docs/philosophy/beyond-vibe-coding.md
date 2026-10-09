# Beyond vibe coding

"Vibe coding" — describing what you want and letting the model
generate it — works great for throwaway scripts, demos, and the
first 80% of a greenfield project. It does not work for shipping
software people depend on. The failure modes are predictable.
Each one has a primitive in oxplow that's there specifically to
catch it.

## Failure mode 1: scope creep

Ask an agent for a small fix and you'll often get a small fix
plus a refactor of three adjacent files plus a "while I was here"
rename. Each piece looks reasonable. The combined diff is
unreviewable.

**Oxplow primitive: the work queue.** A task is a row, not a vibe.
When a work list is active, tasks make scope and follow-ups durable.
New scope can get its own row rather than disappearing into a diff.
Filing is optional; observed turns, changes and evidence are recorded
without requiring a task before an edit.

## Failure mode 2: silent regressions

The agent fixes the thing you asked about and breaks the thing you
didn't. Because the prompt doesn't mention the broken thing, the
agent has no incentive to notice.

**Oxplow primitive: Local History.** Observed changes and agent turn boundaries produce snapshots,
independently of task bookkeeping. When something looks wrong,
you diff the current state against the snapshot from before the
agent started — and you can restore a single file without
disturbing the rest of the working tree.

## Failure mode 3: lost context across sessions

A new session starts with no memory of what the last one was doing,
why this approach was chosen over the alternative, or what was
tried and rejected. The agent re-explores, sometimes re-introduces
the rejected approach.

**Oxplow primitive: durable tasks + wiki pages + threads.**
The task is the persistent intent. Notes on it carry rationale
across turns. When a knowledge store is active, agent guidance encourages retaining
useful Q&A — rationale, comparisons, walkthroughs — with links back to
its evidence. Threads preserve lines of work across sessions. Research
threads can investigate and record understanding without owning
worktree edits. Work-list and knowledge providers are optional.

## Failure mode 4: no review trail

Chat-driven coding leaves no record of *why* a change was made.
The diff exists; the conversation that shaped it does not. Six
months later, nobody knows whether the weird-looking thing in
`auth.ts` was deliberate or a one-shot artifact.

**Oxplow primitive: efforts and notes.** Activity, references and snapshots preserve evidence of the work.
Optional effort policies group that activity and can link it to tasks;
an effort is not a mandatory task attempt. Descriptions and decisions
can connect the observed change to its rationale.

## Failure mode 5: no rollback

When the agent goes wrong, the recovery is `git reset` and hope.
That's fine if the bad change is the most recent one and you
haven't committed since. It's a disaster otherwise.

**Oxplow primitive: file history.** Inspect a captured version and
restore the selected file without rewriting git history or restoring
unrelated files. Review the resulting diff so newer work is accounted
for.

## What "beyond" actually means

It does not mean "stop using agents." It means: keep the speed,
add the structure. The agent still does the typing. The structure
is what lets the work survive past the demo.
