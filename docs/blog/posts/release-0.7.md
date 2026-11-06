---
date: 2026-11-05
categories:
  - Releases
---

# Oxplow 0.7 -- Your agent builds what you see

0.7 is the biggest change in direction so far. We're shipping fewer finished screens and putting the work into a data layer, a UX, and an extension system, so your own agent can build the views and workflows your project needs.

<!-- more -->

## Why the change

Through 0.6 I kept adding pages for questions I guessed people would have. Every project cares about different numbers and different workflows, so I was never going to guess them all, and each new screen was more Rust and React to maintain.

Agents change that math. You can now ask yours for what you want:

> Make me a page of the tasks blocked in this stream.

For a one-off answer, the agent shows a page beside the conversation and stores nothing. If you want to keep it, **Save as Page** or **Save to wiki**. If it grows into something you use every day, it becomes an extension in your repo that you can keep to yourself or share with your team.

For that to work, Oxplow's own screens had to be built the same way. They are now: the app's pages are YAML files in extensions, using the same format and the same checks your agent gets.

## What Oxplow provides now

### A data layer

- Everything Oxplow knows is readable through SQL over published `v_*` datasets: turns, tool calls, commits, snapshots, test runs, coverage, work items, comments, the wiki, and so on. `v_dataset` lists them.
- Collectors bring in outside data as entities and facts, so a GitHub pull request can join to your streams and tasks. They can also classify or summarize with a model through the `ai_*` functions, and those results are recorded.
- Measures, dimensions and metrics from 0.6 now live in extensions, including the ones Oxplow records about itself.
- Every resource has a URI (`oxplow:oxplow/commit?sha=abc1234`). Links, comments, search and agent context all use the same address.
- Every write goes through one command bus, for people and agents alike, with validation, confirmation, audit, and undo where the command supports it.

### A UX

A page is named queries plus a tree of tables, charts, boards, trees and forms, and it re-runs when its data changes:

```yaml
# .oxplow/extensions/review/pages/blocked.yaml
title: Blocked Work
params:
  - { name: thread_id, label: Thread }
answers:
  - { question: "What is blocked?" }
loaders:
  blocked:
    sql: |
      SELECT uri, title, updated_at FROM v_work_item
      WHERE state = 'blocked' AND thread_id = :thread_id
view:
  viz: table
  from: blocked
  columns:
    - { key: title, label: Item, link: { kind: page, from: uri } }
    - { key: updated_at, label: Updated, format: relative, width: 120 }
  empty: Nothing is blocked.
```

The shell gives every page the same behavior: tabs, back and forward, bookmarks, comments, keyboard-driven tables, forms, and **Add to Context** for handing what you're looking at to your agent. An extension can mount a section into another extension's page through a slot, replace a view outright, or draw something custom in a sandboxed component when YAML isn't enough.

### A way to write extensions

The `oxplow-extension` skill ships with Oxplow, so your agent already knows the format. The loop it follows:

```sh
oxplow extension new page my-review     # scaffold one that checks clean
oxplow extension check my-review        # every problem, as file:line
oxplow extension test my-review         # run its fixtures on a throwaway oxplow
```

- Fixtures are `given` / `when` / `expect` YAML. They run against a throwaway Oxplow holding only the data the fixture makes, never your project's database. Oxplow's own pages are tested this way now too.
- `oxplow.extension.deploy` makes an extension live without a restart. If it doesn't load, the old deployment keeps running, and `oxplow.extension.rollback` goes back further.
- Beyond pages, an extension can add collectors, commands, effects that react to events, scheduled and AI jobs, skills, and advisories for your agent.
- Some of Oxplow's core pieces are swappable capabilities. An extension can provide the work list, effort policy, snapshots or wiki store, and external providers can add agent harnesses and filesystems.
- `oxplow extension new --from <example> <name>` starts from one of the [example extensions](https://github.com/nvoxland/oxplow/tree/main/examples/extensions). They include a GitHub pull request collector, an AI review of each finished effort, a TODO triage board, and a replacement task board.

Repository code is untrusted no matter who or what wrote it. An extension declares the permissions it needs, and you approve them when you deploy it. Scripts run in worker processes that are killed at their time or memory limit. There's more hardening to do here; the [extensions reference](../../reference/extensions.md) describes the current limits.

## Where this leaves things

Oxplow now ships two extensions, and they're in different states:

- `oxplow-foundation` is required: the app's own pages, the default work list, effort policy, snapshots and wiki store, and the agent harnesses. We've gone through this one and cleaned it up. Treat it as the base to build on.
- `oxplow-bundled` is optional and on by default: the task and wiki screens, the review packet, analytics, and code metrics. **We haven't gone through this one yet.** It works, but it's a pile of what used to be built in, and the next release will sort it out. Some of it will change or go away.

You can turn bundled off and see what's underneath. Fair warning: without it, work items and wiki links open on a generic resource page. That's intended, but it's bare.

The product is unfinished, but this is a good milestone if building your own views interests you, because the foundation is in the shape I want it in.

## Everything else

There have been about 2,200 commits since 0.6, so this is only the general picture.

Agents:

- Agent harnesses are a provider contract with a conformance suite. Claude Code, Codex and OpenCode all use it, and an extension can add another.
- Codex sessions are Codex's own TUI, on a thread Oxplow opens and observes. Not available on Windows yet.
- A session can list its harness's saved conversations and switch to or fork one, and you can set model and effort at launch for Claude and Codex.
- Subagents are tracked per harness, and Oxplow now works out when an agent is waiting on you, instead of asking the agent to report it.
- The writer thread is gone. It never really protected the checkout, since shell commands weren't checked. A thread is now open or closed, each agent is told who else is in its worktree, and a separate stream is the way to get isolation.

AI models:

- Model access over the API, mapped to roles, with keys in your keychain.
- The Assistant, a short-lived agent in the bottom bar for questions and small tasks. It reads freely and needs **Allow changes** turned on before it can change anything.

Viewing things:

- Markdown, CSV and TSV files, images, PDFs and mermaid diagrams each have a page, as a mode beside the editor.
- The file viewer shows a file at any revision.
- A commit can be undone if it hasn't been pushed.
- **Install oxplow command line...** in the app links the `oxplow` command.
- A lot of performance work, mostly on metrics, pages, search and terminal reattach.

## Upgrading from 0.6

There's no migration plan. Let me know if you need help keeping your project.

Have your agent build something and tell me where it went wrong. The places it gets stuck are what I most want to fix next.
