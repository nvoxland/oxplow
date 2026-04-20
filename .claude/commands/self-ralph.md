---
description: Drive newde through an exploratory or known-bug task, fix what surfaces, log the run
---

# /self-ralph — one (or N) self-driving passes

One invocation = one pass through the loop by default:

> **pick → drive → fix → reflect → log → repeat (on next invocation)**

The loop is *resumable from a cold context* and *portable across
machines*. All state lives in `.self-ralph/` (gitignored), in
`ux-test.md` (tracked), and in `.context/*.md` (tracked). Each pass
must stand on its own.

## What "dogfooding newde" actually means (read first)

Self-ralph is a **user-level** loop. You are the user. The newde app
in front of you is the thing under test. The agents **inside** newde
are the ones with MCP access — they call `mcp__newde__*_work_item`,
commit, approve, etc. on their own when a user (you) prompts them.

**Rules that follow from that:**

- **Never call `mcp__newde__*` tools yourself.** Don't create work
  items, approve commit points, or mutate batches via MCP. If a
  scenario needs that state, drive it through the UI — type in the
  agent pane, click the approve button, drop a file into a batch
  chip. That's the dogfood.
- **Never write to `.newde/state.sqlite` with `sqlite3`** while newde
  is running. You're a user, not a database admin.
- **Prefer using newde over reaching around it.** When you need to
  look at a batch's history, open it in newde's history panel
  (ideally via a probe that drives the UI there) instead of running
  `git log` directly. `git log` is fine for reading your own commit
  trail in the same conversation; it's not fine for answering "what
  does newde show the user about this branch?" — that's dogfood
  territory.

Your two modes of driving newde, in order of preference:

1. **Scenario probes** (`tests-e2e/probe-*.ts`) — Playwright drives
   newde through a full user flow (prompt → wait → approve →
   inspect). Probes are reproducible, hit the real UI, and lock in
   regressions.
2. **Widget probes** — Playwright exercises a single affordance
   (click this, verify that testid). These are fine when you're
   fixing a specific bug but should not dominate the loop. See
   "Guard against the testid rut" below.

## Argument: pass count

The command takes an optional positive-integer argument:
`$ARGUMENTS`.

- No argument (or non-numeric / ≤ 0): run **one** pass and stop.
- A positive integer N: run **N** passes back-to-back. Each pass
  fully completes Steps 1–7 before the next starts, and emits the
  end-of-pass report (Step 8) in between so progress is visible.
  After pass N, emit a final roll-up and stop.

**Stop early if any of these happen** — and tell the user why:

- A pass ends with uncommitted changes you can't explain, or the
  working tree is broken.
- `.self-ralph/todo.md`'s **Next up** section is empty.
- The same root cause shows up in two consecutive passes.
- A probe hangs and the watchdog (Step 3 below) had to kill it;
  file an `[F]` against the harness and move on, but don't retry
  in the next pass.
- The user sends a course correction.

Between passes, **re-run Step 1 (Sync bearings) fresh**. A multi-pass
run must behave identically to N separate invocations from cold.

## Ground rules

1. **Never touch `.self-ralph/` from application code.** This dir
   belongs to the command; newde must stay unaware of it.
2. **Two classes of improvement, different homes.** Problems with
   *newde* (the app) → fix in newde's code/tests/docs, commit, log
   to `.self-ralph/fix-*.md`. Problems with the *harness, this
   command, probes, or your own workflow* → fix directly in
   `tests-e2e/harness.ts`, `.claude/commands/self-ralph.md`, or
   `.self-ralph/`. Never dump meta-friction into newde's codebase.
3. **Red/green TDD still applies** for newde changes. Probe or
   colocated `bun:test` — whichever matches the seam — must be
   green before you commit.
4. **Update the matching `.context/*.md` doc in the same commit** as
   any newde code change — per `CLAUDE.md` rules. Don't skip this.
5. **Commit only newde changes in the "fix" commit.** `.self-ralph/`
   is gitignored. Harness / probe / command changes ARE tracked and
   go in the same commit as the matching fix (or a separate commit
   if purely infra).
6. **You are a user, not an MCP caller.** See "What dogfooding
   means" above — no `mcp__newde__*` calls from /self-ralph, ever.

## Guard against the testid rut

**Testids are probe infrastructure, not newde deliverables.**

- **Never file `[F] add data-testid=...` as a standalone todo.** If
  a probe needs a testid, add it in the same pass that drives the
  scenario. Adding testids detached from a scenario turns the loop
  into mechanical busywork that ships zero user-visible value.
- **At pick time, scan the top 3 of Next up.** If all three are
  testid-adding or one-line UI polish, skip to the first
  scenario-level or infra item instead. Explain the skip in your
  "what I picked" announcement.

## Rotation rule

Every pass whose number is a multiple of 3 (pass 3, 6, 9…) **must
not** be a single-widget polish item. Pick one of:

- **(a) Workflow scenario** — a probe that drives a multi-step user
  flow end-to-end (agent prompt → wait point → approval → commit →
  history). If the matching probe doesn't exist, writing one is the
  pass.
- **(b) IPC / runtime / startup noise** — pick from the
  `### IPC / runtime / startup noise` section of todo.md. These
  items tend to be every user's first impression and the loop
  under-picks them.
- **(c) Cross-cutting consistency** — things like context-menu
  parity, palette population, keyboard-first affordances. File one
  of these proactively if none exists.

Announce which rotation branch you chose. If you can't find one,
say so and pick a normal item; note it in the fix log.

## Step 1 — Sync bearings (cold-start safe)

Run in parallel, and do not read more than this:

- `git log --oneline -5` — recent commits (other /self-ralph passes
  left trails; recognize the Co-Authored-By line).
- `head -40 .self-ralph/todo.md` — the top of the work stack.
- `ls -t .self-ralph/fix-*.md 2>/dev/null | head -3` — skim titles.
  Open one only if its subject is close to what you'll pick.
- `ls -t .self-ralph/review-*.md 2>/dev/null | head -1` — the most
  recent `/ralph-review`. **If one exists, read it fully.** It's
  the tightest summary of what the loop is doing badly right now;
  it names repeat-offender patterns, items that got flagged and
  stayed unfixed, and infra fixes the review itself made. Apply
  those findings — don't repeat mistakes it already called out.
- Also scan the transcript of **this conversation since the last
  `/ralph-review`** (or since session start if there hasn't been
  one). That window is what the next `/ralph-review` will
  evaluate you on. Knowing what's already in it helps you avoid
  redundant work and surface-area blind spots.

**Do not** default-read `.context/*.md` in Step 1. Only read the
specific doc matching the todo item you're about to pick, and only
after Step 2.

If `.self-ralph/todo.md` doesn't exist, stop and tell the user — the
command is mis-installed.

## Step 2 — Pick the top `[E]` or `[F]` item (with guards)

1. Check the "Guard against the testid rut" condition. If it
   applies, skip forward.
2. **Apply the prior-review guidance.** If the most recent
   `/ralph-review` flagged a specific pattern you're about to
   repeat (e.g. "three of the last four passes were testid
   busywork", or "the loop keeps under-picking IPC noise"),
   pick against that pattern, not with it. Name the review's
   finding when you announce your pick.
3. Check the "Rotation rule" — if this pass number is a multiple of
   3, pick from (a)/(b)/(c) regardless of top-of-stack.
4. Otherwise take the topmost item in the **Next up** section. Only
   cherry-pick from lower if the top is genuinely blocked — say so,
   explain why, then take the next one.

Announce your pick **and rotation/guard reasoning** in one
sentence so the user can redirect before you spend tokens.

Then read the single `.context/*.md` that matches the area you're
about to touch.

## Step 3 — Drive

For `[F]` items: go fix it. TDD normally — failing test first when
feasible.

For `[E]` items:

1. **Preflight.** If `dist/` looks stale (`find src -newer
   dist/electron-main.cjs` emits anything), run `bun run build`
   first. The `runProbe()` wrapper in harness also kills stray
   electron processes and stale instance locks on entry, so you
   don't need a separate cleanup step.
2. **Reuse before writing.** Extend an existing
   `tests-e2e/probe-*.ts` when the scenario overlaps. New probes
   are cheap to create and expensive to maintain.
3. **Every probe must use `runProbe("name", main)`** from
   `tests-e2e/harness.ts`. That wrapper supplies:
   - a wall-clock hard timeout (default 90s),
   - a silence watchdog (default 30s — if no `[probe]` line in that
     window, the probe is killed),
   - `[probe:boot]` / `[probe:done]` / `[probe:fail]` markers,
   - stray-electron cleanup.
   Adjust `{ wallMs, silenceMs }` for scenarios that legitimately
   take longer.
4. **Log with `probeLog(...)` or bare `console.log`** — both work;
   the watchdog treats any stdout line as a heartbeat. Emit a
   `[probe] ...` line at each meaningful step so a hang is
   localized.
5. **Run with `node --experimental-strip-types`.** Playwright +
   bun don't mix; see `.self-ralph/README.md` if it exists.
6. **Capture the first concrete friction; don't broaden.** Land the
   fix before chasing the second thing.

**If `runProbe` kills a probe as silent or timed-out:** do not try
again in the same pass. File `[F]` against the harness (or the
newde code if the hang is clearly in newde), write the log, and
stop this pass. Retrying eats tokens without progress — we saw this
eat 25 minutes in a pass-3 incident.

If the probe uncovers user-facing friction, flip the pass from
`[E]` to a fix. Red/green: the probe IS your failing test.

## Step 4 — Fix (when something surfaces)

Edit → rebuild → re-run probe + `bun test` → update the matching
`.context/*.md` doc → commit with a HEREDOC commit message ending
with the Co-Authored-By line.

## Step 5 — Reflect (two hats, tightly)

- **Bot hat:** what about the loop/harness/command was awkward? Fix
  it right here (in `.self-ralph/`, `tests-e2e/harness.ts`, or this
  file) — don't just write it down.
- **Engineer hat:** what about newde's code made the fix harder than
  it should have been? That becomes a new `[F]` or `[E]` todo
  entry.

Do not conflate. Meta-friction is fixed, not logged.

## Step 6 — Update the todo stack

1. Remove the completed item from `.self-ralph/todo.md`.
2. Add new `[F]` / `[E]` items for things you noticed but didn't
   fix. **Do not file testid-adding as a standalone item** (see
   guard above). Roll testid debt into the next scenario that
   needs it.
3. If you filed a new item in the `### IPC / runtime / startup
   noise` section, bump it above the generic editor/file-tree
   section so the rotation rule can reach it.

## Step 7 — Write the log (short template)

Create `.self-ralph/fix-<YYYYMMDD-HHMMSS>-<slug>.md`. Timestamp
format: `date +%Y%m%d-%H%M%S`. Slug = 2-4 kebab-cased words.

**Aim for 20–30 lines total.** The old template produced 60–80
line logs that nobody re-reads. The information density is what
matters.

Required sections (keep each to 1–3 lines unless there's a genuine
surprise):

```
## Picked
<verbatim todo line, why (for [E]: what hypothesis)>

## Shipped
<commit hash + one-line summary, optional `git show --stat`>

## Verification
<probe output line, bun test pass count, or manual check>

## Reflection
<bot-hat + engineer-hat merged, 2–4 lines max>

## Follow-ups
<bulleted new todo entries, or "none">
```

If you have nothing surprising to say in reflection, write "nothing
surprising." That's a valid and preferred answer.

## Step 8 — End-of-pass report to the user

≤ 80 words. State:
(a) which item you picked (+ any rotation/skip reasoning),
(b) what shipped (commit hash),
(c) one-line headline reflection (or "nothing surprising"),
(d) next top-of-stack.

In multi-pass mode, prefix with `Pass k/N:`. After the final pass,
emit a single roll-up line (`/self-ralph N/N complete — shipped:
<hashes>; next top-of-stack is <X>`) and stop.
