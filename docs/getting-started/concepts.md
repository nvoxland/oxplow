# Concepts

Oxplow helps developers understand software and the work agents are doing
on it. Descriptions, connected data, and custom views are the starting
point; files and diffs let you inspect the evidence underneath.

## Stream

A **stream** is one branch and one worktree. The **primary** stream uses
the project directory; additional streams use separate checkouts. This
lets development proceed on different branches without sharing the same
files on disk. Streams and their threads appear in the navigator.

## Thread

A **thread** is a line of work inside a stream, with its own context,
page tabs, and activity. One thread per stream is the **writer**, which
owns worktree edits. Other threads can research and review; their agent
hooks deny worktree writes. This is a workflow guard, not an operating
system sandbox.

A thread can have no agent session or several. Sessions use registered
harnesses, such as Claude Code, Codex, OpenCode, or an ACP connection.
The harness belongs to the session, rather than being a permanent choice
for the thread. See [Agents](../guide/agents.md).

Switching threads restores their tabs. Right-click a thread in the
expanded navigator to **Make writer**.

## Worktree isolation

Each stream has its own directory and branch. Editing a file in one
checkout does not edit the copy in another. This separation protects
parallel work; it does not prevent an arbitrary shell program from
accessing other directories.

## Page

A **page** is anything addressable inside a tab: a description, dashboard,
lens, wiki page, task, file, diff, settings panel, or agent session.
References and backlinks connect these surfaces so you can move from a
summary to its supporting evidence.

## Data, metrics, and dimensions

Collectors bring information into oxplow's shared data model. Extensions
can add their own entities and facts, then join them to existing data.
**Measures** describe collected numbers; **metrics** aggregate them;
**dimensions** let you compare them by project, stream, effort, or other
relevant categories. See [Metrics](../guide/metrics.md).

A **lens** queries that data and presents it as a page or a section of
another page. You can ask an agent to build a lens or collector for your
own questions and share its extension files with your team. See
[Lenses](../guide/lenses.md).

## Task

A **task** is an optional durable work item. The common states are
`todo`, `in_progress`, `blocked`, `done`, and `canceled`; a provider can
have additional native states. Tasks can form epics and dependency
links. Creating or closing a task is not a prerequisite for editing or
ending an agent turn.

The active work-list provider supplies tasks. A project can use oxplow's
implementation, another provider, or none. See
[Work queue](../guide/work-queue.md).

## Wiki page

An optional knowledge provider stores durable explanations and rationale.
The built-in wiki uses markdown under `.oxplow/wiki/`, indexed in SQLite.
`[[wikilinks]]` connect pages to other pages, repo files, commits, and
other references.

When a knowledge store is active, agent guidance encourages capturing
useful understanding there. With no store selected, a question can be
answered without creating a page. See [Wiki pages](../guide/wiki.md).

## Change Analysis

A **change** can be a commit, uncommitted work, or the work within an
effort. Built-in analysis and bundled lenses help explain what changed
and where to look, with links to files and diffs. The presentation is
customizable; see [Change Analysis](../guide/change-analysis.md).

## Effort and snapshot

**Snapshots** record observed working-tree state, including changes made
outside an explicit task. Agent turn boundaries and background capture
provide evidence of what changed.

Optional effort policies group activity into an **effort** and can link
it to tasks. An effort is not simply a task's open-and-close cycle, and
committing its work does not automatically mark the task done. See
[Local History](../guide/local-history.md).

## Backlinks

References connect tasks, knowledge, files, findings, and other entities.
Backlinks let you find the explanations or activity that refer to the
thing you are viewing. Extensions can use these connections to present
richer context than a file or task would provide on its own.

## How they fit together

Open a project, ask questions, and supervise work in threads. Collected
activity, snapshots, analysis, and optional task and knowledge providers
make the work understandable. Add collectors, metrics, dimensions, and
lenses when your questions need more information or a different view.
Drill into code when you need to verify the explanation.
