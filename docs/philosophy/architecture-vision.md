# Architecture vision

Where oxplow is heading, in rough order of conviction. This is a
statement of direction, not a roadmap with dates. Some of this is
shipped today; most is not.

## Today

- **Streams + worktrees.** One agent per branch, isolated checkout,
  no cross-stream writes. This is the foundation everything else
  rests on.
- **Threads inside streams.** Independent lines of work per stream
  — a writer that ships, plus optional read-only research threads
  with their own agents and tab sets.
- **Durable work queue.** Work items as rows with a real lifecycle
  (`ready` → `in_progress` → `done`), ordered by `sort_index`,
  grouped by epic, persisted in SQLite. Filing-enforcement and
  Stop-hook audits keep the queue honest.
- **Local History.** Per-effort file snapshots, grouped under their
  work item, with a page that lets you compare and restore.
- **Project wiki.** First-class markdown notes under
  `.oxplow/notes/`, with `[[wikilinks]]` to other notes, files,
  and commits, plus cross-kind backlinks.
- **Web-style UI.** Pages, tabs per thread, browser-style
  back/forward, kebabs instead of right-click menus, drag-to-add-
  context onto the agent terminal.
- **MCP control plane.** Oxplow exposes its primitives (work items,
  notes, threads, dispatch, LSP, subsystem docs) as MCP tools so
  the agent can drive them directly.

## Near-term direction

- **Richer review affordances.** The work-item-level diff is a
  better unit than the file-level diff. Surface it more
  prominently. Make accept / push-back / reopen one click.
- **Better notes.** A first-class wiki-style notes pane is
  shipping; the next step is treating notes as durable memory the
  agent can consult, not just the human.
- **Tighter LSP and test-runner integration.** The editor knows
  more than the agent does about the project. The agent should be
  able to ask: where is this defined, what tests cover this file,
  what does the type checker think.
- **Smarter dispatch.** Today the user resumes queue work by
  prompting or running `/work-next`; `read_work_options` returns
  either an epic-as-unit or the next standalone cluster.
  Eventually the runtime should propose the next item with
  rationale, not just hand back the head of the queue.

## Medium-term direction

- **Multi-repo streams.** Real systems span repos. A stream should
  be able to coordinate work across more than one of them — same
  branch model, same queue, same Local History.
- **Agent-to-agent handoff via threads.** Today threads are
  read-only-vs-writer. The richer model is: a query thread does
  research, hands a finding to the writer, the writer acts. That
  needs structured handoff, not just "you can also call MCP tools."
- **Common control surface for non-Claude agents.** The MCP layer
  is the right abstraction. Anything that can speak it should be
  able to drive an oxplow stream.

## What's deliberately not on the list

- **A general-purpose chat IDE.** Plenty of those exist. Oxplow is
  opinionated about the workflow, not a blank canvas.
- **Cloud sync, accounts, teams.** Local-first, single-user. If
  multi-user becomes a real ask, it joins the list. Not before.
- **Replacing the editor.** Monaco is the editor. The work happens
  around it.
- **Replacing git.** Git is the source of truth. Streams are
  branches. Commits are user-driven and unchanged. Local History
  is *additive* — it doesn't fight the git model.

## Honest about what's not built

The things in "today" work and are used daily. The things in
"near-term direction" are partially landed; specifics will change
as the work happens. The "medium-term" section is the bet — what
the project is shaped *toward*, even when this week's work is
filling in something smaller.

Direction beats roadmap when the underlying technology is moving
this fast. Oxplow tries to publish the direction honestly and let
the roadmap fall out of it.
