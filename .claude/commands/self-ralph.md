---
description: Drive newde through an exploratory or known-bug task, fix what surfaces, log the run
---

# /self-ralph — one self-driving pass

One invocation = one pass through the loop:

> **pick → drive → fix → reflect → log → repeat (on next invocation)**

The loop is *resumable from a cold context* and *portable across
machines*. All state lives in `.self-ralph/` (gitignored), in
`ux-test.md` (tracked), and in `.context/*.md` (tracked). There's no
"conversation history" requirement — each pass must stand on its own.

## Ground rules

1. **Never touch `.self-ralph/` from application code.** This dir
   belongs to the command; newde itself must stay unaware of it.
2. **Distinguish the two classes of improvement.** Problems with *newde*
   (the app) → get fixed via newde's code/tests/docs and logged to
   `.self-ralph/fix-*.md`. Problems with the *harness, this command,
   the todo process, or your own workflow* → fix them directly
   (harness is `tests-e2e/harness.ts`; this command is
   `.claude/commands/self-ralph.md`; doc is in `.self-ralph/`
   itself). Do NOT dump meta-friction into newde's codebase.
3. **Red/green TDD still applies** for changes to newde code. No
   pre-existing test? Either add one (bun test, colocated) or add a
   Playwright probe under `tests-e2e/`. Tests are what make the fix
   real.
4. **Update the matching `.context/*.md` doc in the same commit** as
   any newde code change — per `CLAUDE.md` rules. Don't skip this.
5. **Commit only newde changes.** `.self-ralph/` is gitignored on
   purpose. Harness / probe / command changes ARE tracked and
   should be in the commit with the matching newde fix (or a
   separate commit if purely infra).
6. **Never touch `.newde/state.sqlite` with `sqlite3` (or any
   other direct writer) while a newde instance is running.** The
   agent driving this command IS a newde instance — it has the DB
   open and cached. Concurrent writes from the CLI can corrupt
   state or be silently overwritten. If you need to inspect or
   mutate the DB directly (e.g. to clean orphan probe rows), do it
   only after confirming no newde is running against this project
   dir, or run the query against a copy. Prefer doing state
   changes through the UI / IPC / MCP tools whenever possible.

## Step 1 — Sync bearings (cold-start safe)

Before anything else, in parallel:

- `git log --oneline -10` — see recent commits (other /self-ralph
  passes have left trails; recognize them by the co-author line).
- `cat .self-ralph/todo.md` — the work stack.
- `ls -t .self-ralph/fix-*.md 2>/dev/null | head -5` — skim titles
  of recent passes to avoid re-doing them. Open one if its subject
  is close to what you're about to pick.
- `cat CLAUDE.md` and any relevant `.context/*.md` for the area the
  top todo item touches. The docs ARE the project memory.

If `.self-ralph/todo.md` doesn't exist, stop and tell the user — the
command is mis-installed.

## Step 2 — Pick the top `[E]` or `[F]` item

Take the topmost item in the **Next up** section of
`.self-ralph/todo.md`. Don't cherry-pick from lower down unless the
top item is genuinely blocked (say so, explain why, then take the
next one).

Announce what you picked in one sentence so the user can redirect
before you start spending tokens.

## Step 3 — Drive

For `[F]` items: go fix it. TDD normally — add a failing test first
if feasible.

For `[E]` items:

1. Build if `dist/` is stale (`find src -newer dist/electron-main.cjs`
   — if there's output, `bun run build`).
2. Write or reuse a probe under `tests-e2e/probe-*.ts`. Prefer
   reusing an existing probe and extending it over creating new
   ones — probes accumulate value.
3. Run it with `node --experimental-strip-types` (Playwright +
   bun don't mix; see `.self-ralph/README.md` if it exists).
4. Capture the first concrete friction. Don't chase more than one
   per pass — land the fix before broadening.

If the probe uncovers friction, flip the pass from `[E]` into a fix
immediately. Red/green: the probe IS your failing test. Land the
fix and confirm the probe flips to OK.

## Step 4 — Fix (when something surfaces)

Standard newde workflow: edit, rebuild, re-run probe + `bun test`,
update the matching `.context/*.md` doc, commit with a HEREDOC
commit message ending with the Co-Authored-By line.

## Step 5 — Reflect

Before writing the log, think explicitly through **both hats**:

- **As a bot driving /self-ralph**: what made the loop awkward? Any
  step where tool output was too noisy, a probe was flaky, a doc
  was stale, a hook blocked something unnecessarily, or the
  todo.md entry was too vague? Those fix the command/infra now.
- **As a software engineer editing newde**: what would have made the
  *actual code fix* faster or safer? Missing testids, no colocated
  test seam, implicit coupling across stores, component too big to
  reason about? Those become new `[F]` items in todo.md.

Don't conflate the two. Meta-friction (slow command, confusing
prompt, stale todo entry) is fixed *here*, in `.self-ralph/` and
`.claude/commands/self-ralph.md` and `tests-e2e/harness.ts`.
newde-friction becomes a todo entry.

## Step 6 — Update the todo stack

1. **Remove** the completed item from `.self-ralph/todo.md`.
2. **Add new `[F]` items** for any newde-friction you noticed but
   didn't fix this pass. Put them at the bottom of **Next up** by
   default, or in **Parking lot** if low priority.
3. **Add new `[E]` items** for unexplored scenarios the probe
   hinted at.

## Step 7 — Write the log

Create `.self-ralph/fix-<YYYYMMDD-HHMMSS>-<slug>.md`. Timestamp
format: `date +%Y%m%d-%H%M%S`. Slug = 2-4 kebab-cased words
summarizing the subject.

Required sections:

- **Picked** — which todo item (verbatim) and why (if `[E]`, what
  hypothesis you were testing).
- **What shipped** — commit hash + one-line summary of code/doc
  changes. Include a `git show --stat <hash>` snippet.
- **Verification** — exactly how you verified: probe output,
  `bun test` pass count, manual check.
- **Bot-hat reflection** — what about /self-ralph itself, the
  harness, or this command's prompt was awkward. Plus what you
  changed in response (or why you didn't).
- **Engineer-hat reflection** — what about newde's code/tests/docs
  made the fix harder than it should have been. These become new
  todo entries — list them here too for traceability.
- **Follow-ups filed** — list of new todo items you appended, so a
  future `git log` + log-file read tells the whole story.

## Step 8 — End-of-pass report to the user

Keep it ≤ 150 words. State: (a) which item you picked, (b) what
shipped, (c) the headline reflection, (d) "next top-of-stack is X"
so the user knows what the *next* /self-ralph will do.
