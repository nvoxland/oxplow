# Pair programming, inverted

Pair programming has two roles. The **driver** has the keyboard:
they type the code, run the tests, fight the syntax, navigate the
file tree. The **navigator** holds the larger picture: where this
change fits, what it might break, when to stop, what to do next.
Good pairs swap roles often, because both jobs are tiring in
different ways.

Agent-assisted development pins one of those roles in place: the
agent is always the driver. You are always the navigator.

## What the agent is good at

- **Typing.** Boilerplate, obvious refactors, mechanical edits.
- **Searching.** Finding every call site, every test, every
  reference, faster than you can grep.
- **Holding local context.** What's on screen, what was just edited,
  what tests are running.
- **Trying things.** Spinning up an experiment, throwing it out,
  trying a different shape.

## What the human is good at

- **Taste.** "This is the wrong abstraction" is still your call.
- **Scope.** Knowing what *not* to do, when "while we're here" is a
  trap, when to stop.
- **Tradeoffs.** Performance vs. simplicity, now vs. later, this
  team vs. that one.
- **Direction.** Where this codebase needs to be in three months,
  and whether this PR moves toward or away from it.
- **Review.** Catching the thing that looks fine in isolation but
  breaks the contract three modules over.

## How oxplow maps onto the split

Each navigator job has a primitive in the product:

| Navigator job | Oxplow primitive |
|---|---|
| Hold the scope | **Work queue** — the durable list of what's in this thread and what isn't |
| Capture rationale | **Wiki notes** — durable Q&A and decisions, with backlinks |
| Pause for taste | **`await_user`** — the agent asks; the runtime suppresses every directive until you reply |
| Catch regressions | **Local History** — every effort snapshots the files it touched |
| Switch contexts cleanly | **Streams** — each agent has its own branch, worktree, and queue |
| Talk without losing your place | **Threads** — query an agent in a read-only thread without risking writes |

You stay in the problem. The agent stays in the keys.

## Why this doesn't reduce to "review the diff"

The naive workflow — let the agent do whatever, then read the diff —
fails for the same reason "throw it over the wall" fails between
teams. By the time you're reading the diff, the design decisions
are baked in, the scope has drifted, and either you accept it or
you start over.

Pair programming worked because the navigator was *present* during
the work, not at the end of it. Oxplow tries to keep you present:
queue first, gates in the middle, history at the end. The diff is
the last line of defense, not the only one.
