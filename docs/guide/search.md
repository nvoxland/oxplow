# Search

One search box over everything oxplow knows: pages, commands, tasks,
threads, knowledge pages, and the files of the active stream's worktree
— by name and by contents, ranked into a single result list.

Open it with ++cmd+p++.

## One launcher, not three

++cmd+p++ is the only entry point. It does four jobs at once:

- **Files** in the active stream's worktree, by name and by contents.
- **Pages** — Tasks, Backlog, Local History, Uncommitted, Git,
  Metrics, Dashboards, and the rest, and each section of a page that
  has them ("Manage services: Integrations" opens on that section).
- **Commands** — the small set of actions that aren't pages, like
  `New Dashboard…`.
- **Content** — knowledge page bodies, task text, and file contents.

Earlier versions had a separate command palette on ++cmd+k++ and a
search palette on ++cmd+shift+f++. Both are gone; keeping three
overlapping pickers meant guessing which one held the thing you
wanted.

## How results are ranked

Everything is ranked on one scale, whatever kind it is. Names come
first, then titles, then contents:

1. The name is exactly what you typed, or a task's id is (`tsk42`).
2. The name starts with it: `dev` finds `DEV.md`.
3. Each word starts a word of the name. camelCase counts, and a word
   can run on: `view` and `markdownview` both find `MarkdownView.tsx`.
4. Each word (three letters or more) is inside the name.
5. Each word starts a word of the title, such as a file's folders:
   `desktop markdown` finds `apps/desktop/…/MarkdownView.tsx`.
6. The words are only in the contents.

Letters scattered through a path don't match: `dev` no longer finds
`apps/desktop/…/InterVariable.woff2`.

Only files belong to a stream. Tasks, knowledge pages and threads are
found whatever stream they are in.

## With an empty query

Opening it without typing shows **Recent** — the last pages you
visited — over a collapsible tree of every page you can open,
grouped by category. That's the page directory: there's no separate
list of pages in the rail.

Tab and Shift+Tab jump between sections.

## Why content search matters here

The point isn't finding a filename you already knew. It's answering
"have we been here before?" — the decision captured in a knowledge page
six weeks ago, the task where this same bug was filed, the note
someone left on that function. Those live in different stores;
searching them separately is how you fail to find them.

See [Knowledge pages](wiki.md) for the durable side of that, and
[Collecting context](context.md) for handing places to an agent.
