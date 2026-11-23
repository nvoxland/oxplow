---
date: 2026-11-23
categories:
  - Releases
---

# Oxplow 0.9 -- Finishing the move to extensions

0.9 finishes the conversion I started before 0.8. Everything Oxplow did before the big refactoring is now an extension, has been merged into one, or is gone. The holding pen for the parts I hadn't reviewed yet, `oxplow-bundled-todo`, is empty and deleted.

<!-- more -->

In 0.8 I said I'd taken the parts of bundled I hadn't gone through out of the app, and that they'd come back one at a time. This release is that work. I went through each piece and gave it one of three outcomes: it shipped in `oxplow-bundled` with fixtures, its ideas went into a page that already existed, or it was removed because something else covers it.

## What came back, in `oxplow-bundled`

- **Change review.** It replaces the old request review. Each change has a Review page with Signals, Tests and Code tabs, and a one-line brief. The signals come from an open list that any extension can add to, and the project sets each kind's severity or turns it off. There's a check for functions a change may have left unused.
- **Git History** has a List view and a Graph view. Search covers the whole history, not only the commits that are loaded.
- **Branch readiness and Remote Branches.** Pushing to a named remote branch now asks first, the same way merging does.
- **Job queue and job output** pages.
- **Agent Interface**: which Oxplow tools and commands your agents use, where their calls fail, and whether advisories reach them.
- **Dashboards, Usage, the code metrics, the Metrics list and Duplicated Code.** These moved out of foundation, and some got improvements on the way.

## What went into existing pages

- **Explore Data** didn't come back. Data catalog, Dataset definition, Dataset data, the Metric page and asking the agent for a page already cover most of it. Two of its ideas did come back:
  - Dataset definition shows lineage, meaning what a dataset reads and what reads it.
  - Any table can be shown as a chart ("Show as"), and Save as Page keeps that choice.
- **The Data catalog** shows one sentence per dataset, so you can scan it. The full text is still on each dataset's page.
- **The two "Extension runs" pages** are gone. Each kind's own page now shows the extension that ran it, what it cost (time, CPU, memory, model calls) and the permissions it used: commands, collectors, effects and jobs. A row opens its run's details.

## What's gone

- Zones, handoffs, plan capture, Timeline, the Assistant and comments. Comments became **Add to Context** (see below).
- The parked advisory rules and their settings.
- The Codex session kind. Codex is an observed session now, like the others.

## Languages are extensions too

Core no longer names a language. Each language is a pack: an extension with the grammar, file kinds, test conventions, run patterns, report collectors and a setup skill.

- Eight packs ship for languages with full support, and they're off until you turn them on. There are smaller packs for languages that only get a language server: Lua, Ruby, Bash and Zig. The JSON, YAML and web packs are on by default.
- A pack can bring a WebAssembly grammar, and `oxplow extension grammar build` compiles one. Clojure's grammar is wasm, and there's an example Elixir pack.

## Run history

Every run of extension code now has one record, `contribution_run`, which holds who ran it, why, what it cost, its permissions and how it ended. Commands, collectors, effects and jobs each keep a detail table that points at it.

- Every command the bus runs gets its own run, reads included. A command that an effect or a job sends is a child run of the one that sent it.
- Collectors keep their run history now, not only the last run.
- A run that was going when Oxplow stopped ends as `interrupted`. Before, it stayed open forever.
- Model calls point at the run that made them.

## Context baskets

You can now select text anywhere and Add to Context: a page, a diff, a terminal. You collect it into named baskets, and a basket goes with a thread's next message. Each item can carry a note. If a message took a basket you didn't mean to send, **Put Back** returns it, and it stays put until you press Send Next.

## Settings

Settings got a rework:
- Choices have labels, and settings have categories.
- Every value says where it came from.
- An extension can edit its own settings from its own pages.
- A value that's refused says where and why.
- AI providers and the model for each role are on one page.

## Everything else

- The agent guidance Oxplow adds to a session is down to a few lines, and the rest is opt-in. oxtask's task workflow is off by default now.
- Restart Terminal restarts an agent's terminal in place.
- The Alerts panel lists each alert as a row you can open and act on. A failed reaction that can't be retried, because its extension is off, no longer stays in Alerts for good.

About 170 commits since 0.8.

## Upgrading from 0.8

The database migrates when 0.9 first starts: the old command audit becomes command runs, and earlier runs get their detail rows. Nothing to do by hand.

`oxplow.zones` is now a retired setting, and Oxplow says so if your project file still has it.

Let me know what breaks. I'd especially like to hear whether the change review's signals are useful or just noise.
