# Search

One search box over everything oxplow knows: tasks, threads, wiki
pages, and the contents of files in the active stream's worktree —
ranked into a single result list.

Open it with ++cmd+p++.

## One launcher, not three

++cmd+p++ is the only entry point. It does four jobs at once:

- **Files** in the active stream's worktree, fuzzy-matched.
- **Pages** — Tasks, Backlog, Local History, Change Analysis, Git,
  Metrics, Dashboards, and the rest.
- **Commands** — the small set of actions that aren't pages, like
  `New Dashboard…`.
- **Content** — wiki page bodies, task text, and file contents.

Earlier versions had a separate command palette on ++cmd+k++ and a
search palette on ++cmd+shift+f++. Both are gone; keeping three
overlapping pickers meant guessing which one held the thing you
wanted.

## With an empty query

Opening it without typing shows **Recent** — the last pages you
visited — over a collapsible tree of every page you can open,
grouped by category. That's the page directory: there's no separate
list of pages in the rail.

Type a task id or an exact page name and it floats to the top. Tab
and Shift+Tab jump between sections.

## Why content search matters here

The point isn't finding a filename you already knew. It's answering
"have we been here before?" — the decision captured in a wiki page
six weeks ago, the task where this same bug was filed, the comment
someone left on that function. Those live in different stores;
searching them separately is how you fail to find them.

See [Wiki pages](wiki.md) for the durable side of that, and
[Comments](comments.md) for the anchored side.
