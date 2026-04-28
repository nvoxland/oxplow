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

## Pair programming, with the roles inverted

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

Streams are oxplow's answer: each agent gets its own branch, its own
worktree, its own work queue, its own threads. You move between them.
They never see each other's writes. Review is per-stream, with Local
History showing exactly what changed in each effort.

## The IDE for the next phase

VS Code was built for one human typing into one repo. That's a fine
tool for the world it was designed for. It's not the world we're
in now.

Oxplow is built for one human directing several agents across
parallel branches of the same repo, with review, rollback, durable
intent, and a project wiki as first-class primitives — not plugins
bolted onto a single-player editor. The UI is web-style: pages,
tabs, links, backlinks. The agent and the human navigate the same
graph.

That's the bet. The rest of these docs are how it shows up in the
product.
