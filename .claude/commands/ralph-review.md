---
description: Review the /self-ralph passes in this session and report honestly on how they went
---

# /ralph-review — honest retrospective on recent /self-ralph work

Look back through **this conversation's transcript since the last
`/ralph-review` invocation** (or since the start of the session if
there hasn't been one). Evaluate the `/self-ralph` passes that
happened in that window.

The point is a brutally honest read, not a victory lap. The loop is
only as good as what it notices about its own failure modes.

## What to examine

Walk the transcript window and pull out:

- Every `/self-ralph` invocation and how many passes it covered.
- The commits produced, by hash + one-line.
- Any pass that was **abandoned** or **stopped early** and why.
- Any moment you spent notable token budget on debugging,
  polling, killing orphan processes, or re-running the same thing.
- Every time you reached for information **outside of newde** (git
  log, grep, direct file reads, sqlite, tests-e2e probes driving
  DOM selectors) when the loop's spirit is to drive newde as a
  user.

Also scan `ls -t .self-ralph/fix-*.md | head` and cross-reference
with commits if needed — the fix logs carry the bot-hat /
engineer-hat reflections and are cheaper to read than the full
transcript.

Don't delegate the synthesis. Read the transcript yourself.

## Structure the report like this

Respond to the user directly (no intermediate file). Headings are
mandatory; length is not. Aim for honest and specific — skip a
section entirely rather than pad it with filler.

### Scoreboard

`N invocations, M passes, K commits shipped, A abandoned, S stopped
early.` One line.

### What went well
The passes you'd defend as actual value. Name commits.

### What went badly
The failure modes. Name them. If a pass looped, eating tokens
without progress, say that clearly. If you spent half a pass
fighting a hung probe, say that. If most of your passes were
testid-adding busywork, say that — the prompt has a rule against
it, and if the rule was broken, call it out.

### How well did you actually dogfood newde?
The `/self-ralph` prompt says "you are a user, not an MCP caller."
- Did you ever launch newde and drive it as a user end-to-end, or
  did you only drive individual DOM widgets through Playwright?
- Did you prompt the agent pane, approve commit points, resume
  from wait points? Or did every probe sit on empty/static state?
- Did you call `mcp__newde__*` tools directly? (You shouldn't
  have. If you did, that's the top problem.)

### Where did you cheat?
Every time you reached outside of newde for information a newde
user would get from the UI. Git log, grep, direct file reads. Some
are fine; some are the loop failing to find the answer inside
newde.

### Token efficiency
Specific cases where you burned tokens:
- Hung probes that took multiple kill/rerun cycles.
- Re-reads of `.context/*.md` files that weren't relevant.
- Over-long fix logs, over-long end-of-pass reports.
- Background command output buffering surprises.

Point at **which prompt rule or harness helper would have
prevented each one.** If no existing rule covers it, propose one.

### Larger patterns you only see in aggregate
Look across the window, not per pass. Usability problems, parts of
newde the loop keeps under-picking, missing affordances,
inconsistencies that no single pass surfaces. These are the
highest-value finds of a retrospective — a pass fixes one thing,
a retrospective names a pattern.

### Bugs you filed vaguely or not at all
Things you *noticed* but either dropped or described so loosely
that a future pass can't act on them. Sharpen them here.

### Follow-up actions

Two lists:

1. **Direct fixes to the harness / prompt / `.self-ralph/`** —
   things you should fix *right now* without a `/self-ralph`
   pass, because they are meta-infra, not newde code. Just do
   them and note what you did.

2. **Recommendations to newde / todo.md** — things that go into
   `.self-ralph/todo.md` (or adjustments to existing entries) so
   future passes pick them up. Avoid the testid-adding-as-todo
   trap from the current prompt's guard.

## After the review

- If you named harness/prompt infra that's broken, **fix it now**
  and commit. The review IS the trigger for those fixes.
- Update `.self-ralph/todo.md` to reflect any new / sharpened /
  outdated items.
- Commit the fixes with a clear message. The review itself is
  ephemeral (it's in the transcript); the resulting code / prompt
  / todo changes are the durable artifact.

## Scope rules

- **Since the last `/ralph-review` only.** Don't re-review ground
  you've already covered. If you see a prior `/ralph-review`
  response in the transcript, start the window immediately after
  it.
- **If no `/self-ralph` passes have happened in the window,** say
  so in one sentence and stop. There's nothing to review.
- **Be honest about your own mistakes.** "I abandoned this pass
  because I couldn't root-cause a hang" is more useful than
  "the pass was exploratory."
- **Never call `mcp__newde__*` tools as part of the review.** You
  are the reviewer, not a newde agent. If the review reveals that
  you previously called them, flag it as the top problem.
