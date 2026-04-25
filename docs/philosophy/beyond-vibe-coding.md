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
The agent is steered by `mcp__oxplow__create_work_item` and the
work-queue lifecycle. If new scope appears mid-flight, it gets a
new row — visible to you, not silently rolled into the current diff.

## Failure mode 2: silent regressions

The agent fixes the thing you asked about and breaks the thing you
didn't. Because the prompt doesn't mention the broken thing, the
agent has no incentive to notice.

**Oxplow primitive: Local History + commit points.** Every turn
snapshots the files it touched. When something looks wrong, you
diff the current state against the snapshot from before the agent
started. Commit points let you mark "things were OK here" without
having to remember to type `git commit`.

## Failure mode 3: lost context across sessions

A new session starts with no memory of what the last one was doing,
why this approach was chosen over the alternative, or what was
tried and rejected. The agent re-explores, sometimes re-introduces
the rejected approach.

**Oxplow primitive: durable work items + threads.** The work item
is the persistent intent. Notes on it carry rationale across turns.
A query thread can ask the writer thread anything without changing
files, so context can be reconstructed without restarting work.

## Failure mode 4: no review trail

Chat-driven coding leaves no record of *why* a change was made.
The diff exists; the conversation that shaped it does not. Six
months later, nobody knows whether the weird-looking thing in
`auth.ts` was deliberate or a one-shot artifact.

**Oxplow primitive: efforts and notes.** Each work item accumulates
efforts (one per attempt) with the files touched, the snapshots
taken, and the agent's own notes. The trail outlives the
conversation.

## Failure mode 5: no rollback

When the agent goes wrong, the recovery is `git reset` and hope.
That's fine if the bad change is the most recent one and you
haven't committed since. It's a disaster otherwise.

**Oxplow primitive: per-effort snapshots.** Restore the state of
just the files an effort touched, without affecting the rest of
the working tree. Targeted undo, not nuclear reset.

## What "beyond" actually means

It does not mean "stop using agents." It means: keep the speed,
add the structure. The agent still does the typing. The structure
is what lets the work survive past the demo.
