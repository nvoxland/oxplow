# Agent control

Oxplow steers the agent through three mechanisms: the **Stop
hook**, the **MCP control plane**, and the **filing-enforcement
hook**. You don't normally touch them directly — but
understanding them is what makes the rest of oxplow's behavior
obvious instead of magical.

## Key invariant

**The runtime never sends prompts to the agent.** The only ways
the runtime steers the agent are:

1. The system prompt set at launch.
2. Hook responses returned over HTTP — especially the Stop
   hook's continuation directive, which tells the active agent
   to keep going.
3. MCP tool responses (when the agent calls one).

There is no auto-typing into the terminal. Auto-progression
through queued work happens because Stop directives keep the
agent from quitting until its obligations are settled.

## Stop hook

When the agent finishes a turn, its generated Claude Code or Codex
integration fires the Stop hook. Oxplow's response decides whether the
agent gets to stop or has to keep working. The pipeline runs in
priority order:

1. **Q&A short-circuit.** If the turn had no qualifying tool
   activity (no edits, no task filing, no dispatch), the
   agent answered or asked a question and is allowed to stop.
   Every directive is suppressed.
2. **Awaiting-user gate.** If the agent has flagged that it is
   waiting on the user, the runtime allows the stop and
   suppresses every directive until the user replies.
3. **In-progress audit.** If the writer thread has any
   `in_progress` tasks, the runtime blocks with an audit
   directive: reconcile each item — still active → leave alone;
   change shipped → close it via `complete_task`; stuck → mark
   `blocked`; obsolete → mark `canceled`. A signature dedup
   prevents the same audit firing repeatedly when nothing
   changed.
4. **Filed-but-didn't-ship advisory.** Catches the misread
   where the agent logged a `ready` row instead of doing the
   work the user asked for.
5. **Effort file-review.** After `complete_task`, the runtime
   diffs the agent's declared `touched_files` against the
   snapshot bracket the effort actually ran against. If the
   sets disagree, a one-shot directive fires asking the agent
   to either `amend_effort` to reconcile or silently agree (the
   prompt won't repeat after agreement).
6. **Otherwise.** Allow stop.

Cross-turn queue progression is **user-driven**. When the agent
finishes its obligations and Stops, it stops — you resume queue
work by sending a new prompt or running the bundled `/work-next`
slash command (which dispatches the next ready cluster).

## Filing-enforcement hook (PreToolUse)

When the agent invokes `Edit` / `Write` / `MultiEdit` /
`NotebookEdit` on the writer thread without an `in_progress`
task, the hook denies the edit before it lands. The agent
files an item at `in_progress` (or flips an existing `ready`
row) and re-issues the edit.

A `ready`-status filing alone does not satisfy the guard.
`ready` is "noticed for later" (backlog); only `in_progress` is
"committed to ship now."

`Bash` is exempt — shell commands routinely mutate the worktree
as a side effect (`git merge`, `git pull`, codegen, formatters)
without representing authored change worth filing.

## Write guard

Each thread has a role:

- **Writer.** Edits files. One per stream.
- **Read-only.** Cannot edit files; can read, search, query.

The role is enforced in the hook. When a non-writer thread
tries to edit, the hook returns an error before the edit runs.
This is non-bypassable from the agent side — the guard is
out-of-process. Promote a different thread to writer (or switch
to the writer thread) if you need to edit.

The wiki directory (`.oxplow/wiki/`) is exempt — wiki capture
works on read-only threads too, because the wiki is research
output, not authored project change.

## MCP control plane

Oxplow exposes its primitives over MCP — tasks, dispatch,
threads, follow-ups, wiki pages, LSP — so the agent can drive
them directly without raw shell escapes. Each thread gets its
own MCP endpoint scoped to that thread, so tool calls implicitly
target the right stream and writer status.

Wiki bodies are an exception: the agent writes them directly
with its built-in `Write` / `Edit` tools rather than through
MCP, which avoids round-tripping full bodies through tool args.
The wiki watcher syncs metadata on every file event.

## What you can change

Most of this is infrastructure — set up once, then invisible.
You can:

- Enable and order Claude Code and Codex for the project (project
  Settings or `.oxplow/project.yaml`). The selected agent is fixed when a thread
  is created.
- Switch the writer thread per stream (thread kebab → "Promote
  to writer").
- Promote a read-only thread when you want it to commit.
- Edit per-stream / per-thread custom prompts (Stream Settings
  / Thread Settings pages).
- Pause or resume a thread's agent process from the thread
  tab kebab.

See [Agents](agents.md) for agent selection and generated runtime
details.

The internal details (hook endpoints, MCP transport, snapshot
storage) are not user-configurable on purpose — changing them
would break the durability guarantees the rest of the product
depends on.

## What's deliberately not here

- **Auto-commit.** The runtime never runs `git commit`. Commits
  are user-driven — run them in the terminal yourself, click
  commit in the Uncommitted Changes page, or tell the agent
  "go run `git commit -m …`".
- **Wait points / commit points.** Removed in favor of
  user-driven commits and the in-progress-audit Stop directive.
- **Auto-progression through the queue.** When the agent
  Stops, it stops. The user resumes by prompting or running
  `/work-next`.
