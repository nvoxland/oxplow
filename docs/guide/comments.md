# Comments

A comment is a threaded note anchored to a specific thing — a range
of lines, a task row, a commit, a chunk of the agent's terminal
output. It stays attached to what it's about, and the agent can read
and answer it.

This is the "ask about it where you found it" surface. You don't have
to break out of what you're reading to raise a question.

## The one thing worth learning first

Every comment has an **intent**, and it decides whether the agent
touches it:

- **Note** — a private thinking note. The agent leaves it alone
  unless you ask.
- **Follow-up** — you want the agent to do something about this.

That split is the whole point. It lets you annotate freely while
reading, without every stray thought turning into work — and lets
you convert one into work later by flipping its intent.

## Leaving one

Select text in the editor, a wiki page, or the terminal and comment
on the selection. Or right-click a row — a file in the tree, a task,
a commit in the graph — and choose **Comment…**.

Comments can anchor to file lines, task rows, terminal and agent-pane
regions, commit rows, the Git dashboard, findings, and wiki text.

If your selection contains references — a file path, a directory, a
commit sha — those are captured alongside the comment, so it knows
what it points at rather than just where it sits.

## They survive edits

A comment stores the text around its anchor, not just a line number.
When the file changes underneath it, oxplow relinks by matching that
surrounding context, within bounds. A comment on line 40 doesn't
silently start pointing at the wrong code because someone inserted
ten lines above it.

Anchors that genuinely can't be found any more are marked orphaned
rather than quietly dropped or relocated to something wrong.

## Finding them again

- **Comments** pane in the rail — open threads at a glance.
- **Comments dashboard** — defaults to unresolved, with resolved
  revealed by date. Every comment has a **Go to location** jump back
  to what it's anchored to.
- **In-page stepping** — move between comments on the page you're
  reading without leaving it.

Threads resolve when they're done, which is what takes them out of
the default view.

## Working them with the agent

Comments are part of the agent's MCP surface, so a follow-up is
something it can pick up, answer inline, and resolve. `/oxplow:review-comments`
walks your open follow-ups and responds to them in one pass — the
review equivalent of working through PR comments, except the code
and the conversation are in the same place.

The useful pattern: read with **Note** intent, decide later which
notes become **Follow-up**, then hand the batch over.
