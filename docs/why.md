# Why Oxplow

Coding agents are good — and getting better fast. They are not,
however, replacing software engineering. They are *relocating* it.
Oxplow is built around what's left for the human once the agent
takes over the typing.

## One-shot vibe coding doesn't ship real software

You can prompt a model into producing a working file. You cannot
prompt it into shipping a maintainable system. Real software has
constraints the prompt can't capture: the integration that broke
the last time someone touched this module, the convention the team
agreed on after a painful incident, the half-built migration the
agent has no way to know is half-built, the architectural direction
that this PR is supposed to nudge toward — not away from.

A one-shot prompt either hits all of that by luck or papers over it
in a way that takes weeks to unwind. The output looks like progress.
It often isn't.

## Agents don't remove software engineering — they relocate it

The bottleneck used to be typing. Reading a spec, holding the model
of the system in your head, translating intent into syntax. Agents
collapse that. The bottleneck is now *steering*: deciding what to
build next, where to draw the boundaries, when to stop, how to
review, what to keep.

Those are the parts that were always hard. They're still hard. The
human now spends *more* of their time on them, not less, because the
typing no longer dilutes the day.

## Pair programming

The closest historical analogue is pair programming — two people,
one keyboard, one driver, one navigator. With an agent on the
keyboard, the human is permanently the navigator. You hold the goal,
the constraints, the architecture, the taste. The agent types,
searches, refactors, runs the tests.

That's a real workflow. It needs real tools. The driver and the
navigator need to share state. The navigator needs to interrupt
cleanly. The driver needs to know when to stop and ask. None of
this is what a chat window is built for.

## Multiple streams, one direction

Once typing is no longer the bottleneck, one human can supervise
several agents at once — *if* the tooling enforces isolation,
captures intent durably, and makes review trivial. Without that, you
get three agents stomping on the same working tree, three threads
of context only one of which you can hold in your head, and a diff
nobody dares to merge.

Streams are oxplow's answer: parallel lines of development use separate
branches and worktrees. Threads organize the work within a stream, and
each thread can have several agent sessions. You move between them,
with linked activity and Local History showing what changed. One writer
thread per stream owns worktree edits.

## Understanding software as the starting point

Oxplow is explicitly not an IDE. It starts with the developer's need to
understand the software and what is happening as agents change it.
Descriptions, questions, evidence, and connected views are the working
surface. Code is available for drill-down when you need to verify an
explanation or make a manual change.

Data collection, analysis, metrics, and dimensions are central to that
approach. Connect changes to tests, development activity to decisions,
or deployment results to the code that produced them. Built-in sources
provide a starting point; you and your agents add what matters to your
project and build the views and workflows you want.

Agents also make personal tools practical. A useful customization can
start with a conversation, become an extension, and be shared with a
team. The foundation should provide the plumbing and enforceable
guardrails so extension authors can focus on their own ideas. Sharing
an extension does not make its code trusted; safety cannot depend on
every version receiving a marketplace review.

That's the bet. The rest of these docs are how it shows up in the
product.
