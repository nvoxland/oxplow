# Your first stream

Open a project, ask what is going on, and follow the evidence. You can
then shape the views around the questions that matter to you.

## 1. Open the project

Launch oxplow and choose the project directory. That directory is the
workspace; oxplow does not climb upward looking for an enclosing repo.
For a source build, see
[DEV.md](https://github.com/nvoxland/oxplow/blob/main/DEV.md).

Oxplow stores local data and runtime files under `.oxplow/`, including
`local.sqlite`, snapshots, and generated agent integration files. The
built-in knowledge provider also uses `.oxplow/wiki/` when selected.

The **primary** stream uses the project directory and its current
branch. Additional streams have separate worktree directories.

## 2. Look at the layout

- **Navigator:** streams and their threads. Expand it to see names and
  right-click actions.
- **Rail HUD:** search, active work, bookmarks, recent pages, and the
  page directory.
- **Page tabs:** descriptions, pages, dashboards, agent sessions,
  tasks, knowledge, files, and diffs. Follow links and backlinks between
  them.
- **Status bar:** branch and background activity.

Start with the view that answers your question. You do not need to open
an editor or create a task first.

## 3. Ask a question

Open an agent session in the thread. If it has none, use **New Agent
Session…** and choose an available harness. A thread can have several
sessions; [Agents](../guide/agents.md) explains configuration.

Try:

> Describe what this project does and how its main parts fit together.
> Link to the code that supports your explanation. Don't change files.

Use the explanation to navigate. When a knowledge provider is active,
agent guidance also encourages saving useful understanding there; it is
not required to answer the question.

## 4. Follow a small change

In the writer thread, ask for a small, reversible improvement appropriate
to your project, such as correcting a README typo. Watch the activity,
then inspect the resulting description and diff.

Tasks are available when you want durable scope and follow-ups. They are
optional: there is no mandatory filing step before an edit. Snapshots
record observed changes, and enabled effort policies can group them and
link them to tasks. Completing an effort does not automatically close a
task.

Open **Local History** from the launcher (++cmd+p++) to inspect captured
versions and compare a file with its current contents. A file restore
changes the working tree; review it as you would any other edit. See
[Local History](../guide/local-history.md).

## 5. Add a stream or thread

Use **New Stream…** to work on another branch in a separate checkout.
The original stream remains available independently.

Use **New Thread…** for another line of work in the same stream. One
thread owns worktree edits; other threads can research or review with
write guards. **Make writer** transfers that role. Agent sessions are
separate from the thread's writer role, so you can choose the harness for
each session.

## 6. Build your own view

Ask your agent for a view over the collected data, such as:

> Build a page showing failed tests by effort, with links to the affected
> changes and the recorded test runs.

**New Page with Your Agent…** in the launcher provides a starter prompt.
The agent inspects the data, writes an extension under
`.oxplow/extensions/<name>/`, and validates it. Open the page like another
page. Share the extension (`oxplow.extension.share`) and commit its files to give the view to your team.

If the data is missing, a collector can add it; metrics and dimensions
can turn it into comparisons over time. Extensions that run programs or
call models need the relevant permissions, which you approve when you deploy them. See [Pages](../guide/pages.md)
and [Metrics](../guide/metrics.md).

## 7. What to read next

- [Concepts](concepts.md) — data, pages, streams, threads, and optional
  work and knowledge tracking.
- [Pages](../guide/pages.md) — custom views, collection, and sharing.
- [Metrics](../guide/metrics.md) — facts, measures, and dimensions.
- [Work queue](../guide/work-queue.md) — optional work items.
- [Agent control](../guide/agent-control.md) — observed activity and
  worktree write guards.
- [Agents](../guide/agents.md) — sessions and harnesses.
