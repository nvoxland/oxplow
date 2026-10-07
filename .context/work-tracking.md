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
tool named them) or *observed* (they changed during one of the thread's
turns and no other thread's overlapping effort claimed them), which covers
shell edits — the large majority. The `effort.observe` consumer records
them per turn from the turn's snapshot bracket, after the policy has
reacted to the turn's `thread.checkpoint`; a file two threads changed at
once and neither claimed is observed by both, and changes between turns
(the person's own) belong to no effort. Runs go to the effort of the tool
call that caused them (`metric_capture.effort_id`). Nothing is declared or
reconciled at close. A commit is linked to the efforts whose changes it
holds, by content.

**A policy decides** when to open, close and link, reacting to core's
events (a checkpoint after a turn's snapshot, a commit indexed, an effort
landed by a commit, a work item changing state). The default:

1. An item started on a thread links that thread's effort, opening one if
   none is open. A descendant refines the link (epic, then child), an
   ancestor leaves it; an unrelated item closes the effort and opens the next; the
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

Built so far: rule 1, in `crate::effort_policy` — a pump consumer on
`work_item.state_changed` (core's, logged for every provider), `effort.linked`
and `effort.opened`. It runs the `effort.*` and `work_item.transition`
commands as the effect `oxplow:effort-policy` and ignores events its own
runs caused. The thread is the event's (the agent that moved the item),
else the item's (`v_work_item.thread_id`); a person moving a backlog item
opens nothing. A descendant of the linked item refines the link; an
ancestor leaves it. The project picks the policy as
`activeProviders.effort_policy`: `oxplow` (the default, unset) or `none`;
both are rows in `v_capability_provider`. A task's status no longer opens
or closes an effort anywhere else.

Rule 2 reads **`thread.checkpoint@1 { thread, turn, reason, snapshot,
changed, writing_tools }`**, logged by the `thread.checkpoint` consumer
(`crate::thread_checkpoint`) once a turn's end take lands. `changed`
compares the turn's start snapshot with the take's (content-addressed:
the same id means the same tree); `writing_tools` counts the turn's calls
to tools that can change the worktree — edits, shell commands, subagents,
`run_command` — from a per-harness name list kept in that module, so no
policy reads tool names. The policy opens an unlinked effort with
`adopt_since` the turn's start when `changed` and `writing_tools > 0` and
the thread has none open; a later item start links it (rule 1).

## No gates

- The edit guard keeps only isolation: a non-writer thread, another
  stream's worktree, wiki pages (written through `knowledge.write_page`).
  It never asks for tracked work.
- The Stop hook never refuses a stop.
- Nothing the agent must call to close work. `effort.report {thread?,
  summary?, impacts?}` is optional and only annotates the thread's open
  (else latest) effort; `v_effort.summary` defaults to the effort's last
  turn's final message.
- "Waiting on you" is derived (a pending question or plan approval, the
  Notification hook's permission prompt, a final message ending in a
  question), not declared; the next prompt clears it. It's the thread's
  logged `agent.status.changed`, read as `v_agent_status`.

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
  Core declares the capabilities (`oxplow_domain::capability`:
  `CapabilitySpec` — choosable, optional, default, the features an
  implementation may declare): the work list and the effort policy may
  be none, snapshots may not, and `vcs` / `knowledge` aren't chosen. A
  person's override is `activeProviders` in `.oxplow/personal.yaml`
  (`config.set { layer: personal }`).
- **No special-casing our own implementations.** Core calls every
  implementation of an interface the same way and never branches on
  whether it is ours or compiled in. Compiled-in, scripted and
  external-process are ways an implementation is *loaded*, not ways it is
  *called*. Our defaults are named built-ins that `oxplow-bundled`
  declares (the pattern collectors use, `entry: oxplow:junit`), use only
  public commands and events, and pass the same conformance suite.
  Core's own records (turns, efforts, the event log) stay transactional
  in core.
  `source_guards::core_never_special_cases_its_own_pieces` fails on a
  literal `"oxplow-bundled"` outside `bundled_extensions.rs` or a
  provider id compared with oxplow's; what's left is pinned with its
  reason (oxplow's tasks' own commands and `dispatching`'s
  in-transaction route, until the tasks sit behind the interface).
- **The agent's tools follow the active implementation.** An interface is
  what oxplow needs to show, link and act; it is not a funnel the agent
  must work through. Each implementation declares its agent surface (its
  commands, its own CLI, or an MCP server the person consents to attach),
  its guidance, and how to recognise its work in what oxplow observes (id
  patterns in prompts, commands and commit messages). Only the active
  one's surface is offered.
- **Disabling `oxplow-bundled`:** optional capabilities fall to none;
  required ones fall back to the capability's default.
- **Switching.** When the registry restates `v_capability_provider` and a
  capability's active implementation differs from the one the rows had,
  it logs `capability.switched@1 { capability, from, to, chosen_by }` in
  the same transaction: once per change, across restarts too, and never
  for the first statement. A chosen external instance that hasn't started
  yet is a real switch to none and back. Switching the effort policy
  closes every open effort (`switch`), whichever policy is active after;
  nothing is deleted.

The three in progress:

| Capability | May be none | Default | Notes |
|---|---|---|---|
| Work list | yes | oxplow's tasks | Task screens stay core components, written against the interface; moving them into the extension is later work |
| Effort policy | yes | the three rules above | An extension policy is an effect reacting to core's events |
| Snapshots | no | keeps everything | Interface: mark now, what changed between two points, read a path at a point (optional, declared as `contents`); a hashes-only implementation ships too |

## Status

- Built: the hook payload dump and the observed hook events; the Stop
  hook never refuses; the edit guard is isolation only; efforts need no
  work item, one is open per thread, and the seam (open with adoption,
  close as of a point, link, retitle — `commands/effort.rs`); the
  effort-policy choice with "none"; rules 1, 2 (`thread.checkpoint`) and
  3 (`vcs.commit.indexed` → `effort.landed`); observed files and runs by
  their causing tool call. An effort's start pin is where its span began:
  the predecessor's end snapshot when it starts at that close, else the
  stream's last snapshot before it when it adopted work already taken.
  Efforts close with their thread (`thread.close`) and stream
  (`stream.archive`, end snapshot first); `effort.report` is optional and
  never creates an effort, and the summary defaults to the last turn's
  final message. "Waiting on you" is derived; `await_user` is gone.
  The Work panel shows the thread's open effort even when unlinked (In
  progress) and its closed ones (Finished, which opens Thread activity);
  the Thread activity lens lists the thread's turns under the effort each
  fell in, with question-only turns "Between efforts"; an effort's page
  header renames, links, unlinks and closes it.
  Hints, first cut: advisories gain a `turn-end` trigger, every param
  (`:thread_id`, `:stream_id`, `:turn_id`, nullable `:effort_id`),
  `once_per: thread`, and one delivery path — every hit is a nudge the
  next prompt or tool call takes and stamps; bundled `large-uncommitted`.
  Second cut: `audience: person` (Alerts, `hint.dismiss`), `once_per:
  session | day`, a per-hook character budget, evaluation counts
  (`v_hint_stat`) and muting after three deliveries; bundled
  `landed-in-progress` to the person.
  Agent text follows: the runtime skill is a short optional guide, the
  dispatch protocol (`dispatch_task`, the brief, the subagent skill) is
  gone, and a retired skill leaves installed runtimes on their next
  write.
- Capability framework, first commits: core declares the capabilities
  (`oxplow_domain::capability`), `.oxplow/personal.yaml` layers a
  person's choices over the project's, extensions declare
  implementations (`implementations:`, built-ins by entry), and one
  `CapabilityRegistry` resolves the active one (personal → project →
  default; an unavailable choice falls to none, or a required
  capability's default) and restates `v_capability_provider` (v3).
  Lenses and advisories declare `needs:`; unmet, a lens says what it
  needs instead of showing empty, and an advisory doesn't run.
  Settings → Pieces, generated from `v_capability_provider` (v4 carries
  each capability's title and whether it's choosable and optional),
  chooses for the project and just for me, and replaces Integrations'
  work-items radio. A switch is logged (`capability.switched@1`) and
  closes open efforts when it's the policy's; a source guard pins core's
  remaining special cases.
- Next: the three
  swappable pieces. Loose
  refs (`tsk12` for a work item) wait for it: recognising an id is the
  active work list's declaration, not core's.

