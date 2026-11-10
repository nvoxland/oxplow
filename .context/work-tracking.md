# Work tracking: inferred, not declared

How oxplow knows what the agent is doing and has done, and the direction
it is moving in. This is the target design; each section says what is
built. Read it before touching requests, turns, the edit guard, the Stop hook,
hints (advisories), task trackers or snapshots.

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
for harnesses where a block never reaches the agent. Any wall-clock or tokens the
tracking costs must buy something the person or the agent uses.

**Order of remedies:** infer first, hint second, gate never. A hint that
keeps firing is a sign oxplow should infer the thing itself.

## The record

The **turn** (prompt to stop) is the unit of record; each **tool call** is
the evidence inside it. Per turn oxplow keeps what opened it (`source`: a
person's prompt, a background job reporting back, a subagent's hand-back,
another session's message, or unknown) with the text it was opened with,
the turn it goes on with when the harness named one (`cause_turn_id`: the
turn whose tool call started the job, or in which the subagent started),
the harness's own turn id, the agent's final message, the start and end
snapshots, the tool calls with their time windows, test runs and tokens
([agent-model.md](./agent-model.md) "Hook ingest and recording"). Only a
person's prompt is a person's words: since 2026-10-28, 41% of turns were
opened by one, 41% by a background job's report, 13% by another session.
A **request** is the turn that started something and every turn that
followed from it, joined by those cause links when each turn opens
([agent-model.md](./agent-model.md) "Requests").

**A turn's file changes are a join by time** (`v_turn_file`), not
snapshots the turn owns: snapshots are the worktree's own record, and
several writers (agents, the person, outside tools) can change one
worktree at once. The join is stored (`turn_file`, V21), written by a
trigger when a take is logged (`snapshot_op`, after its file rows): a
take that changed nothing logs none. A file is the turn's when the take
captured it after the turn began, while it ran or within a minute after
it ended, and the watcher saw it change inside the turn's window; a
capture with no change time (the startup sweep's) counts only while the
turn ran or in the turn's own end take. A file the watcher saw change
before the prompt is not the turn's, even when the turn's end take
captured it. `shared` for a capture with no change time also looks at the
minute before it (another turn's end take lands up to a minute after that
turn closed). Computed per
read, the join over every turn and file took tens of seconds whenever a
query named turns by anything but a constant id; over the table a filter
on a turn, a request's turns or a stream and path reaches an index. Each file says how sure it is:
`claimed` (a call of the turn named it, or its harness reported a shell
call changed it: `agent_tool_call_path`), `shared` (another turn ran on
the stream while it changed), `sole`. `thread.checkpoint`'s `changed` is
whether the turn has any; the `turn` and `request` change kinds find their
two snapshots by time (the end is the turn's own `turn_end` take, else the
last take at or before it ended). Nothing here depends on how many threads or agents share a stream.

`OXPLOW_HOOK_DEBUG=<file>` appends every hook payload, as the agent sent
it, to that file, one JSON line each (`event`, `thread`, `at`, `payload`).
Use it to learn a harness's real payload shapes before depending on them.
Besides the hooks oxplow acts on, the Claude Code plugin registers events it
only observes — `TaskCreated`, `TaskCompleted`, `PreCompact` — which are
acked unread until something needs them
(`crates/oxplow-harnesses/src/claude.rs` `HOOK_EVENTS`). `SubagentStart`
and `SubagentStop` are read: see "Who in a session acted" in
[agent-model.md](./agent-model.md).

## Requests, files and assignment

oxplow groups work by **request**, not by a bucket the agent or a policy
opens: a request is the turn that started something and every turn that
followed from it ([agent-model.md](./agent-model.md) "Requests"). Work
recorded against a turn — tool calls, tokens, test runs, jobs — is the
request's through the turn. Its state (`running`, `waiting_on_person`,
`waiting_on_work`, `finished`) is read live, so nothing opens or closes it.
There is no policy to choose and nothing to link by hand.

**Attribution is core's, from observation.** Files are the turn's by the
time join above (`v_turn_file`), and a file is `claimed` when an edit tool
named it (`claim_basis` `named`) or the harness said a shell call changed
it (`reported`: Claude Code's `bashEditDiff`, never taking a named file; its
absence says nothing). The rest are `sole` or `shared`, which covers shell
edits — the large majority. A claimed file and a tool call carry the agent
session that made them (`agent_session_id`), so two sessions on one thread
are told apart past the door. Runs go to the turn of the tool call that
caused them. Nothing is declared or reconciled at close.

A commit is linked to the requests whose changes it holds, per path, by
content (`commit_request`: basis named/reported/observed, confidence
sole/shared; [git-integration.md](./git-integration.md) "Linked to the
requests that made it"). A task is linked to the requests that worked on it
(`request_task`, `.context/tasks.md` "A task's requests"): each link has
its basis (`prompt`, `command`, `transition`, `assigned`) and is written
when its evidence is recorded. The task → commit edge derives through the
task's requests.

**Assignment is core's** (`task_assignment`, [tasks.md](./tasks.md)
"Assignment"). An agent that starts a task (`oxplow.task.transition` to in
progress), or files one straight into progress naming no assignee
(`create`), takes it: the command assigns it to the agent's thread in its
own transaction, whoever had it. There is no effort policy and no
`oxplow.effort.*` command. Nothing in oxplow marks a task done, or moves
one, because of a request.

Evidence is anchored to requests too: metric deltas and observations
(`v_request_metric_delta`, `v_request_observation`), a request's outcome,
and per request tokens, cycle time, time to green and test outcome.
Efforts were removed (2026-11-10); steering and token waste went with
them.

## Cross-thread ownership

Who is going to work on an item is its assignee — a thread, the person or
nobody (`v_task.assignee`; `oxplow.task.assign`, core's for
every list, [tasks.md](./tasks.md) "Assignment") — and there is
no second one. Who worked on it is its requests (`v_request_task`, below). A done item is nobody's,
and a closed thread's items go back to nobody. Two threads may
still work on the same thing, and nothing refuses it (below, "No
gates"). It is made visible, all of it bundled over existing datasets
(oxplow-bundled):

- The **Shared work** page (`shared-work` in the launcher under Work;
  `thread-shared-work` on the thread page, slot `thread.plan.header`;
  `item-shared-work` on a task, slot `oxplow/task-detail.sections`): items
  that requests on two threads worked on (`v_request_task`; requests
  running, or finished in the last day), files two threads' turns changed
  (`v_turn_file` over those requests' turns, only a `claimed` or `sole`
  file: one two concurrent turns could each have changed is no evidence),
  and epics whose open children sit on different threads.
- The advisory **`shared-files`** (turn end, to the person, once per pair
  of threads): requests on different threads, running or lately finished,
  changed the same file (`v_turn_file` of their turns, `claimed` or
  `sole`) within `sharedFiles.windowMinutes`
  (default 60). It names both threads and the files. Either thread's turn
  end can raise it; an Alert already raised for the pair (its text opens
  with the pair, `v_agent_nudge`) silences the other, so a long overlap
  alerts once.
- The advisory **`shared-files-agent`** (turn end, to the agent, once per
  thread): another thread's request also changed the files, with a
  pointer to the handoffs to or
  from that thread (`v_handoff`; `oxplow.handoff.send` hands the overlap over).

None refuses anything. Work handed from one thread to
another is a handoff ([agent-model.md](./agent-model.md) "Handoffs and
native messaging"): taking one moves its item.

## No gates

- The edit guard keeps only isolation: another
  stream's worktree (the primary checkout included), wiki pages (written through `oxplow.knowledge.write_page`).
  It never asks for tracked work.
- The Stop hook never refuses a stop.
- Nothing the agent must call to close work. A request ends when its
  turns do; its last turn's final message is the summary.
- "Waiting on you" is derived (a pending question or plan approval, the
  Notification hook's permission prompt, a final message ending in a
  question), not declared; the next prompt clears it. It's the thread's
  logged `agent.status.changed`, read as `v_agent_status`.

## Hints

Hints replace gates. They are **advisories** (`advisories.rs`,
`advisories:` in `extension.yaml`), one mechanism: a query over the
datasets, a trigger, pacing, and an audience.

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
  ("fewer tasks filed"), which invites filing to satisfy a number. What a
  task tracker wants of its agent (oxtask's `untracked-work` reminder to
  take a task before editing) is that tracker's opinion and lives in the
  tracker's extension (`oxplow-bundled`), not in core's hints; it fires
  once per request, after the first edit, only while no task is in
  progress on the thread and none is linked to the request.

## Review: evidence in core, policy in extensions

Core collects and analyzes; whether the result is good enough is a
review policy's, and a policy is an extension. Core owns the evidence
(test runs and cases with their provenance, coverage and diff coverage,
the change analysis, metric facts), its
attribution to turns and requests, its freshness, schema validation and the safe
operations that write it. A policy reads that evidence through the `v_*`
datasets, adds facts of its own (collectors, measures, dimensions), and
says what it concluded through public contributions: a dataset, a page, a
decorator, commands.

oxplow-bundled and oxplow-foundation are the default policy. Another
replaces them piece by piece: `examples/extensions/schema-review` is a
whole alternative review built that way.

| Judgement | Where it lives | Knob |
|---|---|---|
| Struggle (a file edited 5+ times, 3+ failed shell commands) | oxplow-bundled `struggle` | `struggle.repeatedEdits`, `struggle.failedCommands` |
| Per-request test outcome and time to green | oxplow-foundation's test-outcome collector | metric specs over its facts |
| Look here first, co-change surprise, deviation, tests weakened | oxplow-bundled datasets and pages | `changeInterest.*`, `coChange.*`; the rest fork |
| Metric thresholds (coverage 80/50, complexity > 10, length > 60) | oxplow-foundation metric specs | `metrics: [{ use: …, target, warnAt, failAt }]` |
| Advisories (15-file uncommitted, threshold crossed, landed in progress, two threads on one file) | oxplow-bundled advisories | `largeUncommitted.files`, `sharedFiles.windowMinutes`; the rest fork |

**What stays in core, and why.** These are collection's own mechanics
or operational limits, not a view of what good work is:

- Which shell commands are test or analysis runs (built-in patterns plus
  `testing.runPatterns`), and the report freshness window.
- The report-less-run nudge: collection telling the agent its run wrote
  no report it could read.
- Which paths and functions are tests (`change_analysis::is_test_path`,
  `is_test_function`) and the assertion and skip counting
  (`test_signals`): conventions the change analysis records as columns.
- Function metrics, clone detection and its `min_lines` / `min_nodes`
  floors, diff-coverage math, zone matching, `threshold_state`.
- Advisory muting after three deliveries, stall detection (15 / 5
  minutes), contribution disabling after three failures.

**Knobs.** An extension's SQL reads its own configuration
(`config('/pointer')`, `.context/extensions-manifest.md` "Configuration in SQL"),
so the constants are settings, not forks. oxplow-bundled has `changeInterest.*`
(weights and the long-function cut-off), `coChange.*` (pair threshold,
window, per-commit file cap, dormant days) and `struggle.*`, set with
`oxplow.review_policy.write`. The holder (`extensions-bundled.md`) has
`largeUncommitted.files` and `sharedFiles.windowMinutes`, set with
`oxplow.advisory_policy.write`. All default to today's values. A metric's `target` / `warnAt` / `failAt` stay
tunable from `project.yaml`.

**Gaps.** The complexity and length cut-offs of the metric specs and the
"look here first" reason threshold (a factor of 1.2) are still fixed in the extension.

## Capabilities

The direction for most of oxplow: a base interface in core, a standard
implementation shipped in `oxplow-bundled`, and users free to write their
own — heavier ones included (beads as a task tracker).

- **A capability** is defined by its interface, whether it may be
  **none** (then a core no-op stands in — for the task tracker a sink that
  takes every verb and keeps nothing, so everything written against the
  interface just finds nothing — and anything that declares it needs the
  capability itself says so), its
  default, its conformance suite, and the kinds of implementation it
  accepts. Capabilities are independent and many: Settings lists them
  from the registry, agents propose changes through config like any
  other, and a choice is a project default with a personal override.
  Core declares the capabilities (`oxplow_domain::capability`:
  `CapabilitySpec` — choosable, optional, default, the features an
  implementation may declare): the task tracker and
  knowledge may be none, snapshots may not, and `vcs` isn't chosen. A
  person's override is `activeProviders` in `.oxplow/personal.yaml`
  (`oxplow.config.set { layer: personal }`).
- **One active, or many at once.** Most capabilities have one active
  implementation, chosen (the task tracker, snapshots) or
  fixed (`vcs`). A **many-capability** (`CapabilitySpec.many`:
  `agent_harness`, `ai_provider`) is served by every
  implementation declared — the way collectors are — never chosen,
  never none and never switched: each declared row is active
  (`chosen_by = declared`), its `default` is the one used when nothing
  names one, and a need on it is met while any is declared, a need on one
  of its features while any of them has it (`Active::unmet`). Their
  implementations are `oxplow-foundation`'s declarations (core's
  built-ins, looked up by key: an agent session's `harness`).
- **No special-casing our own implementations.** Core calls every
  implementation of an interface the same way and never branches on
  whether it is ours or compiled in. Compiled-in, scripted and
  external-process are ways an implementation is *loaded*, not ways it is
  *called*. Our defaults are named built-ins that `oxplow-bundled`
  declares (the pattern collectors use, `entry: oxplow:junit`), use only
  public commands and events, and pass the same conformance suite.
  Core's own records (turns, requests, the event log) stay transactional
  in core.
  `source_guards::core_never_special_cases_its_own_implementations` fails on a
  literal `"oxplow-bundled"` or `"oxplow-foundation"` outside
  `bundled_extensions.rs` or a provider id compared with oxplow's; what's
  left is pinned with its reason. oxtask sits behind the interface
  like any tracker (`crates/oxplow-oxtask`, registered by declaration, its
  verbs called outside the transaction — [tasks.md](./tasks.md)).
  The agent harnesses likewise: `only_harness_implementations_name_a_harness`
  fails on production code outside `crates/oxplow-harnesses` and
  `crates/oxplow-harness-fake` that names a harness's own pieces
  (`AgentKind`, its CLI's flags, env markers, metric names, tool names,
  `CLAUDE.md`, `.claude/`), with a pinned list (`HARNESS_IMPLEMENTATION`,
  empty) for any that must stay, each with its reason
  ([agent-model.md](./agent-model.md) "Launching the agent").
- **The agent's tools follow the active implementation.** An interface is
  what oxplow needs to show, link and act; it is not a funnel the agent
  must work through. Each implementation declares its agent surface (its
  commands, its own CLI, or an MCP server the person consents to attach),
  its guidance, and how to recognise its work in what oxplow observes (id
  patterns in prompts, commands and commit messages). Only the active
  one's surface is offered.
- **Disabling `oxplow-bundled`:** optional capabilities fall to none;
  a required one keeps its default, which is core's own (registered by
  the registry, never declared by an extension).
- **Switching.** When the registry restates `v_capability_provider` and a
  capability's active implementation differs from the one the rows had,
  it logs `capability.switched@1 { capability, from, to, chosen_by }` in
  the same transaction: once per change, across restarts too, and never
  for the first statement. A chosen external instance that hasn't started
  yet is a real switch to none and back. Nothing is deleted.

The three in progress:

| Capability | May be none | Default | Notes |
|---|---|---|---|
| Task tracker | yes | oxtask | Task screens stay core components, written against the interface; moving them into the extension is later work |
| Snapshots | no | keeps everything (core's `oxplow`) | `SnapshotProvider` (`oxplow_domain::snapshot`): mark now, what changed between two points, read a path at a point (bytes only with `contents`). `Services.snapshots` (a `SnapshotRegistry`) answers the active one; every core take (a turn's end, a measured run, an archive) is its mark. Both built-ins are `snapshots::CoreSnapshots`, one capture pipeline under a content policy: `oxplow` keeps every version, `hashes` (bundled, "Track changes only") records identities and keeps no bytes. The captures follow the active one's policy (`SnapshotSwitch` on `capability.switched`, and every reload); a switch changes what's kept from then on, nothing taken before. A provider process implements it too (`providers::snapshots::SnapshotsHost`, `.context/providers.md` "A snapshots provider"): it marks, core records what it reports in the same ledger, and the capture pipeline idles while it's active |
| Knowledge | yes | oxplow's wiki (bundled, `oxplow:wiki`) | `KnowledgeProvider` (`oxplow_domain::knowledge`): write, delete and link pages, and how current each page's pinned refs are. `Services.knowledge` (a `KnowledgeRegistry`) answers the active one. None is a sink, as the task tracker's is (`knowledge::NoneKnowledge`, core's, always registered): a write lands nowhere and succeeds, so nothing that writes pages is refused. An extension's provider may keep pages elsewhere (an Obsidian vault, a Confluence or MediaWiki server; `.context/knowledge.md` "The capability") |

## Status

- Built: the hook payload dump and the observed hook events; the Stop
  hook never refuses; the edit guard is isolation only; requests, with
  their state read live; files per turn by a time join with the snapshot
  line (`v_turn_file`); runs by their causing tool call; assignment taken
  by the agent that starts a task. "Waiting on you" is derived;
  `await_user` is gone. (Today the Work panel shows tasks: In progress
  with what the thread's latest request is doing, Up next, and Finished —
  the done tasks the thread was working, `v_request_task` basis assigned
  or transition.) Efforts were removed (2026-11-10).
  Hints, first cut: advisories gain a `turn-end` trigger, every param
  (`:thread_id`, `:stream_id`, `:turn_id`, nullable `:request_id`),
  `once_per: thread` (since then `request`, the default, pacing per
  request), and one delivery path — every hit is a nudge the
  next prompt or tool call takes and stamps; bundled `large-uncommitted`.
  Second cut: `audience: person` (Alerts, `oxplow.hint.dismiss`), `once_per:
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
  Pages and advisories declare `needs:`; unmet, a page says what it
  needs instead of showing empty, and an advisory doesn't run.
  Settings → Capabilities, generated from `v_capability_provider` (v4 carries
  each capability's title and whether it's choosable and optional),
  chooses for the project and just for me, and replaces Integrations'
  task tracker radio. A switch is logged (`capability.switched@1`), and
  a source guard pins core's
  remaining special cases.
  The agent's surface follows the active implementation: commands
  declare `needs:`, built-ins declare the commands and tools only they
  offer, an instance owns its namespace, and the bus and MCP offer and run
  only what's active. Skills and slash commands come from extensions
  too (`skills:`), offered the same way; the task guidance and
  `/next-task` moved from core into `oxplow-bundled`. A task tracker
  declares its id pattern, and a loose id in a command (`tsk12`) is the
  active list's item.
  The app's test fixtures (`services_with_thread`, `services_with_task`)
  make a thread, and a task only for tests about tasks.
- Next: "none" as a task tracker end to end, then the other two
  capabilities in the table (snapshots, knowledge).
