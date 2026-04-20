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

## What "dogfooding newde" actually means (read first — THIS IS THE WHOLE POINT)

Self-ralph is **newde improving newde** through actual use. You, the
outer agent, are a **human engineer** sitting at a newde window.
When you want a fix, you don't open an editor — you **tell the inner
agent inside newde to do it**, by creating a work item and prompting
the agent pane. You watch the inner agent work, you approve the
commit through the UI, and you **notice every friction that slowed
you down**. Those frictions are the raw material of the loop.

If you finish a pass and you never launched newde, never typed into
the agent pane, never approved a commit through the UI — **you did
not dogfood, and the pass was a failure** regardless of what code
got committed. The `/ralph-review` at `20260419-213605` explicitly
caught this failure mode: 5 passes, 0 dogfood. Don't repeat it.

### The canonical pass, spelled out

1. **Launch newde yourself**, via the electron app running from this
   repo. You can script the launch with Playwright (`launchNewde`
   from `tests-e2e/harness.ts`) but you are driving as a user: you
   click the buttons a user would click, you read the text a user
   would read, you notice the delays a user would notice.
2. **Prompt the inner agent in the agent pane terminal** — this is
   your primary channel. Type a clear task description directly
   into xterm: "Pick up this todo: <verbatim line>. Here's the
   scope: <pointers>. Run bun test after, propose a commit." The
   inner agent has MCP tools; it will create its own work items to
   plan, surface a commit point when ready, etc. **You prompt; it
   executes.** You are not allowed to call `mcp__newde__*`.
3. **Do not pre-stack work items via the Plan UI.** The `+ New work
   item` button exists, but work items are mostly the *inner
   agent's* planning artifact and your window into what it did. If
   you preemptively fill the Plan queue with items the agent hasn't
   started yet, you're acting like a PM, not a user — and you're
   front-running the inner agent's own decomposition. Exception:
   one work item per pass is fine if you want the Plan UI to
   reflect "the thing I'm dogfooding right now." Don't queue up
   pass 2's work in pass 1.
4. **Watch.** Snapshot the terminal rows periodically. Read what
   the inner agent is doing. Notice when the UI tells you nothing
   about progress. Notice when a wait point lands without surfacing
   in the Work panel. Notice when an approval button is hidden
   under a hover card. **These are the findings.**
5. **Approve the commit through newde's UI** — not via `git
   commit`, not by editing a file. Click the approve button. If
   you can't find it, that's a finding.
6. **Reflect on friction.** Everything that surprised you, slowed
   you down, or required reaching outside newde goes into the
   fix log and/or a new todo entry.

`tests-e2e/dogfood-cycle.ts` is the reference implementation of
this flow. It's the canonical probe; treat its structure as the
template for any new scenario. (Note: that probe does create one
work item before prompting, because the prompt references it by
title — that's the acceptable one-item exception in step 3, not
a license to pre-queue.)

### Hard rules

- **Never call `mcp__newde__*` tools yourself.** Those are for the
  inner agent. You prompt it; you don't bypass it.
- **Never write to `.newde/state.sqlite` with `sqlite3`** while
  newde is running. You're a user, not a DB admin.
- **No reaching around newde.** When you need to look at a batch's
  history, open it in newde's History panel. When you want to see
  what changed, open the diff in newde's editor. `git log` is fine
  for reading your own commit trail in the conversation; it is not
  fine for answering "what does newde show the user about this
  branch?" — that's dogfood territory. Cheating through Grep and
  Read is easier than driving the UI, which is exactly why the
  friction you'd notice never gets noticed.

### When Playwright is and isn't appropriate

Playwright is a **user-simulation harness**, not a shortcut around
user-level interaction:

- **Primary use (dogfood):** script the canonical pass above — the
  agent-loop scenario end-to-end. The probe drives newde as a user
  would: creates a work item, prompts the agent, approves the
  commit. This is a full dogfood run captured as a regression test.
- **Secondary use (regression lock-in):** after a dogfood pass
  surfaces a bug and you fix it, a narrower widget probe can lock
  in the fix. This is *derived* from a dogfood finding, not a
  substitute for one.
- **Failure mode to avoid:** writing a widget probe *instead* of
  doing the dogfood pass, because the widget probe is easier.
  That's the 2026-04-19 session's whole mistake. If the first
  thing you reach for is a Playwright probe that drives a specific
  testid without ever creating a work item or prompting the inner
  agent, stop — you're about to repeat the failure.

### Attestation requirement

Every end-of-pass report (Step 8) **must include a one-line
"Dogfood:" field** stating what you actually did as a user. Valid
answers name concrete UI interactions — "launched newde, created
work item via +New, prompted inner agent in terminal, approved
commit via Work panel." The string "none" or "n/a" is a valid but
**damning** answer and should only be used for pure infra-only
passes (harness / prompt / .self-ralph/ edits), which must be
disclosed as such in "Picked."

### The user-experience mindset (non-negotiable)

Dogfood isn't just "drive the happy path." It's **using the
product the way a real user would — curiously, fallibly, with
expectations** — and writing down every moment reality didn't
match what you expected. Three disciplines make this real:

**1. Narrate expectations BEFORE acting.** Every non-trivial UI
action gets a one-line "I expect X to happen when I click Y"
written in the fix log or probe commentary *before* the click.
The delta between expectation and observation is the finding.
Examples of expectations worth naming:

- "I click + New work item — I expect a modal with title,
  description, and a focused title input."
- "I click the Commit (5) button — I expect a dialog showing
  only my changed files, with the option to unstage untracked
  ones."
- "I press Cmd+K and type 'history' — I expect one row, View ›
  History."

If what you expect is different from what you see, **that's a
bug**. File it. Don't rationalize ("oh, I see why they did it
that way") — a user's first read is the test.

**2. Scan for capability before reaching outside newde.** When
you think "I need to see the git log" or "I need to check what
changed," pause and look at newde's UI first. What's on the
screen? Does a History panel, a Changes tab, a blame overlay,
a Cmd+K command already do this? If you could use newde for
this and chose not to, that's a cheat — and usually a finding
(the affordance was there but wasn't discoverable enough that
you noticed it under pressure). Log the near-miss.

**Specific surfaces you should actively use — not just know about:**

- **Bottom pane → History tab.** The canonical "what did we
  ship?" view. Use it instead of `git log` when verifying the
  inner agent's commit landed (see Step 7 of the canonical pass).
  It also shows turn-level history with changed-files trees —
  a richer view than git gives.
- **Bottom pane → Snapshots tab.** Per-turn snapshots of the
  working tree. Cross-reference against History when you want to
  see what a specific turn changed.
- **Bottom pane → file history.** When a file is open in the
  editor, its per-commit history view is the answer to "when
  did this line change?" — faster than `git log -p <file>` if it
  works well. Open it at least once per session.
- **Files pane filter modes.** `ProjectPanel` surfaces filters
  along `branchBase`, `upstream`, `uncommitted`, and the current
  batch's file changes. Try switching filters — they're the
  newde-native answer to `git diff --stat HEAD` and
  `git diff origin/main...HEAD`. Notice if a filter doesn't do
  what its label suggests.

**Check-in rule.** Even when the task doesn't need History,
Snapshots, or filter modes, **open each of these panes once
during every dogfood pass** and confirm it shows sensible
content. "Sensible" means: reflects recent commits, loads
within a second or so, doesn't render an empty-state when
there's obviously state. Any of these failing in passing — slow
load, empty render, inconsistent count, broken filter —
becomes a Surprises bullet. The point is not to re-verify
every pass; it's that **these are the surfaces most likely to
rot silently** because they're read-only and no test pings them
on a normal flow.

**3. Spend 60 seconds exploring unrelated surfaces.** At some
point during each dogfood pass — ideally after the inner agent's
commit lands but before you write the fix log — **poke at three
UI elements you didn't need for the task**. Open a panel you
haven't used. Right-click something. Try a keyboard shortcut
you're not sure of. Log anything surprising:

- An empty state that's misleading.
- A hover card you didn't know existed.
- A button whose title reveals behavior the label hides.
- A response time that feels slow.
- A state that rendered wrong after a drag/drop/approval.

The point is NOT to fix everything you find. The point is to
NOTICE. "Surprise" bullets go in the fix log's Friction section
alongside the task-related findings. If you found nothing
surprising, write "nothing surprising in 60s of exploration" —
but scrutinize whether you actually explored or just scanned the
Work panel again.

**Why this matters.** Without these, dogfood degenerates into
"script the happy path, approve the commit, call it done." The
loop then ships fixes but doesn't discover the usability
regressions that any real user would hit in minute one. The
2026-04-19 22:16 review flagged this: 3 dogfood passes shipped,
8 frictions surfaced — but all 8 were task-local. Zero came
from unrelated exploration.

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

## Guard against the subsystem rut

**One file or directory shouldn't own 3+ consecutive commits.** The
`/ralph-review` at `20260419-213605` caught four consecutive passes
landing in `src/ui/commands.ts` + `src/ui/components/CommandPalette/`;
each pass was individually legitimate but the aggregate starved the
rest of the app.

- **At pick time, run `git log --oneline -3` and check the files
  touched.** If the last 3 commits all touched the same single file
  or the same top-level directory under `src/ui/components/`,
  force-pick an item from a different area. Announce the redirect.
- This guard sits *above* the top-of-stack rule — it's a blast-
  radius concern, not a priority concern. The topmost item comes
  right back on the following invocation.
- It does **not** trip on harness/prompt/probe commits; only newde
  code. Infra commits are meta and shouldn't count against
  newde subsystems.

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

## Step 3 — Drive (dogfood first, Playwright second)

**The default shape of every pass is a dogfood run**, whether it's
flagged `[E]` or `[F]`. You launch newde, create a work item, prompt
the inner agent, watch, approve. The fix itself should be done by
the inner agent through newde, not by you in your editor. Skip this
only if the item is harness/prompt/infra — and disclose that in the
"Picked" announcement.

### 3a. Preflight
If `dist/` looks stale (`find src -newer dist/electron-main.cjs`
emits anything), run `bun run build` first. The `runProbe()`
wrapper kills stray electron processes and stale instance locks on
entry, so no separate cleanup step.

### 3b. The dogfood run (canonical — do this unless it's infra)

Write or extend a probe modeled on `tests-e2e/dogfood-cycle.ts`:

1. `launchNewde(projectDir)` into a fresh userData dir.
2. Click through to the UI area you're exercising (Work panel,
   editor, History, etc.) as a user would. Notice every delay,
   missing affordance, or unclear label on the way.
3. **Click `+ Commit when done` first** (the
   `plan-add-commit-point` button on the `plan-add-points-bar`).
   Without an active commit point, `propose_commit` silently
   no-ops and the agent will report "no active commit point
   existed" only in its terminal output — not in the Work panel.
   Surfaced by the 2026-04-19 22:02 dogfood run (`fix-
   20260419-220229-ctxmenu-dogfood.md`).
4. **Focus the agent pane xterm and prompt the inner agent
   directly.** That's the canonical channel. Your prompt names
   the task verbatim, gives scope / file pointers / acceptance
   criteria, and asks the agent to run tests and propose a commit
   when done. Press Enter.
   - Optional: if you want the Plan UI to reflect the active task
     for your own visibility, create **one** work item via `+ New
     work item` *before* prompting and reference it by title in the
     prompt. That's it. Don't pre-queue a backlog.
5. **Watch.** Poll every ~10-15s: screenshot, dump `.xterm-rows`
   innerText to a file. Heartbeat with `probeLog()` each tick so
   the silence watchdog stays happy. Use a long `wallMs` (e.g.
   10 * 60_000) and matching `silenceMs` (e.g. 90_000) — the
   agent can be quiet while thinking. Reference template:
   `tests-e2e/dogfood-ctx-menu.ts`.
   - `[data-agent-status]` doesn't work as a signal — there's a
     filed `[F]` against newde for it. Rely on terminal content
     until that lands.
6. **Approve via the UI.** When the agent surfaces a propose
   result, the Work panel should offer an approve affordance.
   If the agent no-op'd `propose_commit` (check terminal for
   "no active commit point existed"), fall back to the
   `files-commit` palette command (Cmd+K → "Commit") and commit
   the agent's changes manually. Either way, the commit lands
   through newde's UI — not via `git commit`.
7. **Verify through newde, not git.** Open the History tab in the
   bottom pane and confirm the commit appears there as a user
   would see it. If the commit touched files, open one of them
   and try the per-file history view. Also flip the Files pane
   filter to "uncommitted" (should be empty after approval) and
   to "branchBase" / "upstream" (to see this pass's delta in
   context). If any of these is awkward, slow, stale, or shows
   less than you expected, it's a finding. `git log` is the
   cheat — don't default to it.
8. **60-second exploration.** Before writing the fix log, poke
   at three UI elements you didn't need for the task. Suggested
   targets if you're not sure where to look: the Snapshots tab,
   a Files-pane filter you haven't tried, or a context menu on
   a row type (history row, blame row, backlog item) you didn't
   touch for the task. See "The user-experience mindset" above.
   Log anything surprising.

Everything that slowed you down in steps 2–8 is a finding, as is
every expectation that didn't match reality. Those go into the fix
log under "Expected vs actual" and "Friction" and become new todo
entries. Don't skip this reflection — it's the whole point.

### 3c. Regression lock-in (derived, secondary)

**Only after** a dogfood pass has surfaced a concrete bug and you
(or the inner agent) have fixed it, a narrower widget probe can
lock in the fix. Reuse `tests-e2e/probe-*.ts` when the scenario
overlaps; new probes are expensive to maintain.

### 3d. Probe hygiene (applies to both 3b and 3c)

- **Every probe must use `runProbe("name", main)`** from
  `tests-e2e/harness.ts`. Supplies wall-clock timeout (default
  90s), silence watchdog (default 30s), `[probe:boot]` /
  `[probe:done]` / `[probe:fail]` markers, and stray-electron
  cleanup. Tune `{ wallMs, silenceMs }` up for dogfood runs.
- **Log with `probeLog(...)` or bare `console.log`.** The watchdog
  treats any stdout line as a heartbeat. Emit a `[probe] ...` line
  at each meaningful step so a hang is localized.
- **Run with `node --experimental-strip-types`.** Playwright + bun
  don't mix.
- **Inside `page.evaluate`, raw CSS only.** Playwright's Locator
  syntax (`:has-text`, `:visible`, `>>`) is library-level, not CSS.
  They silently fail as invalid selectors and throw from
  `querySelector`. For text or role matching inside the browser,
  iterate `document.querySelectorAll("button")` and filter on
  `.textContent`. Use `window.getByRole(...)` / `getByText(...)`
  at the Playwright API level instead. Surfaced by
  `review-20260419-221646.md`.
- **Capture the first concrete friction; don't broaden.** Land the
  fix (via inner agent) before chasing the second thing.

**If `runProbe` kills a probe as silent or timed-out:** do not try
again in the same pass. File `[F]` against the harness (or newde if
the hang is clearly in newde), write the log, stop this pass.
Retrying eats tokens — saw this burn 25 minutes in the pass-3
toggleBlame incident.

If a dogfood run uncovers user-facing friction, that IS the finding
— file it, prompt the inner agent to fix it, and approve through
the UI. Red/green still applies: the friction is the failing test,
the fix is when the friction is gone.

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

## Dogfood
<concrete user actions: launched newde, created work item "X",
prompted inner agent with "Y", approved commit at <sha> via Work
panel. OR "infra-only pass — no dogfood needed because <reason>".
Bare "none" without justification is a failed pass.>

## Expected vs actual
<the expectations you named before acting, paired with what
happened. Each bullet: "expected X → got Y" — the delta is the
finding. At least two bullets if this wasn't an infra pass.
Example:
 - expected + Commit dialog to show only tracked changes →
   actually bundled untracked probe files too.
 - expected agent-pane scrollback to be empty on fresh launch →
   actually showed prior session's transcript including
   reverted commits.
If literally every expectation matched, say so — but that's rare
enough to be worth double-checking whether you had expectations
at all.>

## Surprises (60-second exploration)
<three things you poked at outside the task's direct path, with
what was notable about each. Examples:
 - Right-clicked a completed batch chip: no "Reopen" option,
   only Rename/Add-batch/Add-stream. Intentional or gap?
 - Opened History panel while uncommitted: the timeline showed
   "working" state correctly but turn-diff loading spinner
   hung for 2+ seconds on a 12-file turn.
 - Pressed Tab in the agent pane xterm: no tab-completion; cursor
   just inserted a literal tab. Is that intended?
If you didn't explore, write "skipped — <reason>". Skipping is
OK if the pass was infra-only, otherwise it's a sign the pass
was too task-focused.>

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
surprising." That's a valid and preferred answer. **"Dogfood" and
"Friction" cannot be empty without disclosure** — those are the
load-bearing sections.

## Step 8 — End-of-pass report to the user

≤ 90 words. State:
(a) which item you picked (+ any rotation/skip reasoning),
(b) **Dogfood:** one line naming concrete UI actions you took as a
    user — "launched newde, created work item, prompted inner agent,
    approved commit." Bare "none" means this was an infra-only pass
    AND you must justify that in (a).
(c) **Top surprise or expectation miss:** the single most
    interesting finding from Expected-vs-actual + Surprises. If
    nothing was surprising, say so — but infrequently.
(d) what shipped (commit hash),
(e) next top-of-stack.

In multi-pass mode, prefix with `Pass k/N:`. After the final pass,
emit a single roll-up line (`/self-ralph N/N complete — shipped:
<hashes>; next top-of-stack is <X>`) and stop.
