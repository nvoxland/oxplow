# Work tracking: inferred, not declared

How oxplow knows what the agent is doing and has done, and the direction
it is moving in. This is the target design; each section says what is
built. Read it before touching efforts, the edit guard, the Stop hook,
hints (advisories), work-item providers or snapshots.

## Why

oxplow used to make the agent declare its work: an edit was refused until
a task was in progress, a stop was refused while one was open, and closing
needed a summary plus a hand-typed file list. Measured on our own
sessions, that cost about 20% of tool calls and 10% of round trips; 12% of
the tracking calls failed on their input shape; 13% of stops were refused,
mostly while the agent was correctly waiting on background work. Yet
oxplow already records turns, tool calls, snapshots, test runs, tokens and
commit links with no help from the agent.

So: **infer, don't enforce.** oxplow works out what is going on; the agent
is never required to do bookkeeping. This must hold for weak models and
for harnesses where a block never reaches the agent (ACP shows a Stop
directive to the person, never the agent). Any wall-clock or tokens the
tracking costs must buy something the person or the agent uses.

**Order of remedies:** infer first, hint second, gate never. A hint that
keeps firing is a sign oxplow should infer the thing itself.

## The record

The **turn** (prompt to stop) is the unit of record; each **tool call** is
the evidence inside it. Per turn oxplow keeps the prompt, the agent's
final message, the start and end snapshots, the tool calls with their
time windows, test runs and tokens. Nothing here depends on one writer
thread per stream.

`OXPLOW_HOOK_DEBUG=<file>` appends every hook payload, as the agent sent
it, to that file, one JSON line each (`event`, `thread`, `at`, `payload`).
Use it to learn a harness's real payload shapes before depending on them.
Besides the hooks oxplow acts on, the Claude plugin registers events it
only observes — `SubagentStart`, `SubagentStop`, `TaskCreated`,
`TaskCompleted`, `PreCompact` — which are acked unread until something
needs them (`crates/oxplow-plugin/src/lib.rs` `HOOK_EVENTS`).

## Efforts: a core bucket, a pluggable policy

An **effort** is a span of one thread's work, the bucket files, runs,
tokens, the review packet and commit links hang off. It is finer than a
commit and needs no task.

**Core owns** the entity and its invariants: spans on one thread never
overlap and at most one is open per thread; the work-item link is
optional and can be attached later; a title (default: the linked item's
title, else the first prompt line); how it closed (`commit`, `switch`,
`person`, `agent`). Core's commands carry the boundary, not the call time,
so a policy may react late: opening adopts the thread's activity back to
its previous effort's end, and closing can cut the span at a past point.

**Attribution is core's, from observation:** files are *claimed* (an edit
tool named them) or *observed* (they changed in the span and no other
thread claimed them), which covers shell edits — the large majority. Runs
go to the effort of the tool call that caused them. A commit is linked to
the efforts whose changes it holds, by content.

**A policy decides** when to open, close and link, reacting to core's
events (a checkpoint after a turn's snapshot, a commit indexed, an effort
landed by a commit, a work item changing state). The default:

1. An item started on a thread links that thread's effort, opening one if
   none is open. An ancestor or descendant refines the link (epic, then
   child); an unrelated item closes the effort and opens the next; the
   linked item going done or canceled closes it.
2. A turn that changed the worktree, and ran at least one tool that isn't
   a read, opens an effort if none is open. A question-only turn never
   gets one; a person's own edits never open one.
3. A commit that lands all of an effort's changes closes it. A partial
   commit links it and leaves it open; a reset or branch switch never
   closes one.

oxplow moves a `todo` item to in progress when work is linked to it, and
**never marks anything done**. The person can rename, link or close an
effort afterwards. A policy may be **none**: no efforts, and the record is
per turn.

## No gates

- The edit guard keeps only isolation: a non-writer thread, another
  stream's worktree, wiki pages (written through `knowledge.write_page`).
  It never asks for tracked work.
- The Stop hook never refuses a stop.
- Nothing the agent must call to close work. `effort.report {summary?,
  impacts?}` is optional; the default summary is the last turn's final
  message.
- "Waiting on you" is derived (a pending question or plan approval, the
  Notification hook, a final message ending in a question), not declared.

## Hints

Hints replace gates. They are **advisories** (`advisories.rs`,
`advisories:` in `extension.yaml`), one mechanism: a query over the
models, a trigger, pacing, and an audience.

- Triggers include **turn end**: evaluated at stop, delivered at the next
  turn start, the one delivery point every harness supports.
- Queries can read trends over time (`metric_grid`), so a hint can fire on
  "pieces of work are getting larger", not just on one row.
- Every delivery is recorded, stamped when the agent actually receives
  it. A hint whose condition keeps holding after repeated deliveries is
  muted and raised to the person: a hint that changes nothing is pure
  cost.
- Trigger on outcomes the person feels (work too large to review, work
  landed while its item still says in progress), never on tracker usage
  ("fewer tasks filed"), which invites filing to satisfy a number.

## Swappable pieces

The direction for most of oxplow: a base interface in core, a standard
implementation shipped in `oxplow-bundled`, and users free to write their
own — heavier ones included (beads as a work list).

- **A capability** is defined by its interface, whether it may be
  **none** (then a core no-op stands in, and anything that declares it
  needs the capability says so instead of showing empty results), its
  default, its conformance suite, and the kinds of implementation it
  accepts. Capabilities are independent and many: Settings lists them
  from the registry, agents propose changes through config like any
  other, and a choice is a project default with a personal override.
- **No special-casing our own implementations.** Core calls every
  implementation of an interface the same way and never branches on
  whether it is ours or compiled in. Compiled-in, scripted and
  external-process are ways an implementation is *loaded*, not ways it is
  *called*. Our defaults are named built-ins that `oxplow-bundled`
  declares (the pattern collectors use, `entry: oxplow:junit`), use only
  public commands and events, and pass the same conformance suite.
  Core's own records (turns, efforts, the event log) stay transactional
  in core.
- **The agent's tools follow the active implementation.** An interface is
  what oxplow needs to show, link and act; it is not a funnel the agent
  must work through. Each implementation declares its agent surface (its
  commands, its own CLI, or an MCP server the person consents to attach),
  its guidance, and how to recognise its work in what oxplow observes (id
  patterns in prompts, commands and commit messages). Only the active
  one's surface is offered.
- **Disabling `oxplow-bundled`:** optional capabilities fall to none;
  required ones fall back to the capability's default.

The three in progress:

| Capability | May be none | Default | Notes |
|---|---|---|---|
| Work list | yes | oxplow's tasks | Task screens stay core components, written against the interface; moving them into the extension is later work |
| Effort policy | yes | the three rules above | An extension policy is an effect reacting to core's events |
| Snapshots | no | keeps everything | Interface: mark now, what changed between two points, read a path at a point (optional, declared as `contents`); a hashes-only implementation ships too |

## Status

- Built: the hook payload dump and the observed hook events.
- Next, in order: the Stop hook stops refusing; the edit guard drops the
  tracked-work rung; the effort schema and seam; the default policy's
  three rules; observed files; waiting derived; the Work panel and a
  Thread activity page; hints; skills and repo rules. Then the capability
  framework and the three swappable pieces.
