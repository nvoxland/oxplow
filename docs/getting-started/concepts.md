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
harnesses, such as Claude Code, Codex, or OpenCode.
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
page, knowledge page, task, file, diff, settings panel, or agent session.
References and backlinks connect these surfaces so you can move from a
summary to its supporting evidence.

## Data, metrics, and dimensions

Collectors bring information into oxplow's shared data model. Extensions
can add their own entities and facts, then join them to existing data.
**Measures** describe collected numbers; **metrics** aggregate them;
**dimensions** let you compare them by project, stream, request, or other
relevant categories. See [Metrics](../guide/metrics.md).

A **page** queries that data and presents it as tables, charts and forms,
alone or as a section of another page. You can ask an agent to build a page or collector for your
own questions and share its extension files with your team. See
[Pages](../guide/pages.md).

## Task

A **task** is an optional durable record of work. Each task store declares its
own stages, and puts each one in a bucket oxplow understands: `backlog`,
`todo`, `in_progress`, `blocked`, `done`, or `canceled`. The screens show
the list's stages; oxplow acts on the buckets. Tasks can form epics and dependency
links. Creating or closing a task is not a prerequisite for editing or
ending an agent turn.

A task store supplies tasks. A project can use oxtask,
another task store, or none. See
[Tasks](../guide/tasks.md).

## Knowledge page

Knowledge stores keep durable explanations and rationale.
Oxplow's own store, oxwiki, uses markdown under `.oxplow/oxwiki/`, indexed in SQLite.
`[[wikilinks]]` connect pages to other pages, repo files, commits, and
other references.

When a knowledge store is enabled, agent guidance encourages capturing
useful understanding there. With no store enabled, a question can be
answered without creating a page. See [Knowledge pages](../guide/wiki.md).

## Change Analysis

A **change** can be a commit, uncommitted work, or the work of a
request. Built-in analysis and bundled pages help explain what changed
and where to look, with links to files and diffs. The presentation is
customizable; see [Change Analysis](../guide/change-analysis.md).

## Request and snapshot

**Snapshots** record observed working-tree state, including changes made
outside an explicit task. Agent turn boundaries and background capture
provide evidence of what changed.

A **request** is the turn that started something plus every turn that
followed from it. It can link to tasks and commits. A request is not
simply a task's open-and-close cycle, and committing its work does not
automatically mark the task done. See
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
pages when your questions need more information or a different view.
Drill into code when you need to verify the explanation.
