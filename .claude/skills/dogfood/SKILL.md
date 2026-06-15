---
name: dogfood
description: Use oxplow like a real user to get work done while QA/design-reviewing it — drive the in-oxplow agent as the navigator, file bugs as oxplow tasks, document findings in ideas/qa. Use when the user says "dogfood oxplow", "use oxplow as a user", "navigator session", or asks for a QA/design-review session of oxplow itself.
---

# /dogfood — navigator session against the real app

You are a senior software engineer using oxplow to ship real work,
**and** a QA engineer + designer reviewing it as you go. The agent
INSIDE oxplow (claude/codex/opencode in a thread pane) is the driver;
you are the navigator. You do not edit project files directly for
feature work — you brief the driver, watch it through oxplow's own
surfaces, and verify what lands. Direct edits are allowed only for
infrastructure that blocks the session itself (e.g. the daemon won't
serve the browser).

## Boot the stack (browser-driven)

Playwright cannot attach to the Tauri shell on macOS; the supported
route is the daemon + a real browser:

```bash
cargo build -p oxplow-daemon
VITE_OXPLOW_REMOTE=http://127.0.0.1:7420 bun run --cwd apps/desktop build
./target/debug/oxplow-daemon --project . --bind 127.0.0.1:7420 &   # background it
bun run --cwd apps/desktop preview &                               # dist/ on :4173
curl -s http://127.0.0.1:7420/health                               # expect "ok"
```

Then drive `http://localhost:4173` with the Playwright MCP tools
(`browser_navigate` / `browser_snapshot` / `browser_click` /
`browser_evaluate`). Prefer `browser_snapshot` + `data-testid`
selectors; screenshot when looks matter, and read the image.

- The daemon holds the per-project instance lock — the desktop app
  can't run against the same project simultaneously. If startup says
  "project already open", `lsof .oxplow/instance.lock` to find the
  holder before assuming staleness (it's an advisory flock; only a
  live process can hold it).
- You own the servers: restart freely. Rust change → rebuild + restart
  the daemon (leave ~3s between kill and start or the boot wiki scan
  can hit the old process's WAL lock). Frontend change → rebuild with
  the same `VITE_OXPLOW_REMOTE` and reload the browser. No HMR by
  design — restart at points you choose.
- A daemon restart relaunches thread agents with `--resume`; a big
  session shows claude's resume picker (answer it in the pane). A
  stale resume id prints `[oxplow] saved resume id was stale; starting
  a fresh Claude session` — normal, the work is in commits/tasks.
  Don't restart immediately after `/clear` — the resume id updates
  on the next hook event, so a restart in that window resurrects the
  cleared session.

## The navigator loop

1. **Brief the driver in its terminal pane.** Click the thread, then
   `focus()` `.xterm-helper-textarea` **immediately before** sending
   (focus gets stolen on re-render). For long text use `fill()` (lands
   as one bracketed paste) then Enter, Enter (the second submits);
   `keyboard.type` with embedded newlines mis-fires. **Keep every
   brief a SINGLE paragraph** — multi-paragraph (blank-line) pastes get
   their paragraphs REORDERED over the web→PTY path (known bug). Ignore
   the greyed *ghost autosuggest* text in the prompt; it isn't in the
   buffer and won't submit. You are a HUMAN at the keyboard: drive by
   typing/pasting into the terminal — never call oxplow's terminal I/O
   transport (`send_terminal_message`) directly to inject a prompt, and
   never script oxplow to generate agent input. oxplow must not automate
   the agent; the agent only ever receives human keystrokes (and makes
   its own tool calls). Write real briefs: scope, constraints, file-the-task-first, tests +
   fmt + clippy, commit per concern, update `.context/` docs.
2. **Wait without burning context.** Done-signal = idle AND stopped.
   `list_hook_events {"threadId":...}` is **newest-first** — check
   `data[0].kind == "stop"` (NOT the last element), AND
   `list_agent_statuses` `state == "idle"`. Back it with a backgrounded
   **Monitor** that emits one line per thread when both hold; the
   dot/status alone lies (see death, step 7).
3. **Work concurrently while it runs.** Explore other surfaces, file
   bugs, test features (comments, search, dashboards), brief a second
   thread, or run a second stream in parallel. Useful IPC for
   inspection: `get_thread_work_state` (`{"threadId":"thr1"}`),
   `list_threads` (`{"streamId":"str1"}`), `list_task_efforts`
   (`{"itemId":"tskNN"}`), `list_effort_observations`
   (`{"effortId":"effNN"}`), `create_task`
   (`{"req":{"threadId":...,"input":{"title","description","parent_id":null,"status","priority","author":null}}}`).
4. **Review what landed** through oxplow itself: Git Dashboard →
   commit pages (zones/treemap/co-change), the Tasks page, per-effort
   "Coverage & tests"/static-analysis panels. Read the driver's final
   terminal summary.
5. **Verify live, never trust the claim.** Rebuild + restart the
   affected half, reload, and exercise the actual fix in the browser
   before considering it done. Verify a suspected bug in source before
   filing — half my "bugs" dissolved on a careful read.
6. **`/clear` the driver when the queue is settled** and the session
   is huge — all context lives in the task system, so re-brief the
   fresh session with task ids only ("work tskNN, tskMM in this
   order; read each description").
7. **If the driver looks stuck, suspect death, not thought.** A
   transient API or model error (e.g. `Claude Fable 5 is currently
   unavailable`) drops it to its prompt while the status stays
   **"Working" for ages** — the turn never closed (no Stop on death).
   Tell: **zero `pre_tool_use` events since the last
   `user_prompt_submit`**. Reading the terminal tail or **reloading the
   page** reconciles the status to "Stalled — agent stopped responding
   mid-turn". Re-dispatch on a healthy thread/model; nudge with one
   message naming where it left off (uncommitted files, failing check).

## Driving agents efficiently (hard-won navigator tricks)

- **Type like a human; never automate the agent.** Drive only by
  simulating keystrokes into the web terminal (Playwright `keyboard.type`
  / `fill()`+Enter+Enter, single-paragraph briefs, re-`focus()` first).
  Do NOT call `send_terminal_message` (oxplow's terminal *I/O transport*
  for human keystrokes/paste) directly to inject a prompt, and never have
  oxplow generate agent input — that would be automating the agent and
  risks violating the agent tool's license. The terminal is flaky (focus
  theft, ghost autosuggest, paste reordering); work around it as a human
  would, don't bypass the human-input path.
- **Real clicks, not synthetic.** `page.evaluate(() => el.click())`
  often doesn't fire React handlers, so UI buttons silently no-op. Use
  a real `page.mouse.click(x, y)` (coords from `getBoundingClientRect`)
  or the Playwright `browser_click` ref. (A button that "does nothing"
  is usually this — confirm via the daemon log / an IPC call before
  blaming the product.)
- **IPC arg shapes.** Wire fields are **camelCase** (`streamId`,
  `itemId`, `effortId`, `targetKind`, `targetId`); command args wrap
  under their param name (`create_task` → `{"req":{...}}`). A
  missing/misnamed **Optional** field silently becomes `None` — e.g. a
  stream-scoped git op with the wrong `streamId` casing silently runs
  on the PRIMARY worktree and returns a misleading "success". Error
  messages are self-correcting; read them.
- **Refs churn.** Rail / branch / tab refs change on every reload and
  often between snapshots — re-snapshot right before clicking.
- **Reconnect = manual reload.** A WS blip shows `Connection restored —
  reload to resync state` with a Reload button
  (`[data-testid="remote-banner-reload"]`); the UI does NOT auto-resync.
  Click Reload (it recurs, especially around bursts of git-ref/snapshot
  events — e.g. merges + concurrent commits). Expect a few reloads/session.
- **Filing-hook exemptions for YOUR edits.** The PreToolUse filing hook
  blocks project-file Edit/Write without an in_progress item — but
  **Bash is exempt** (write QA notes + scratch via Bash heredoc) and so
  are edits made **mid-merge/rebase** (MERGE_HEAD present). For real
  navigator infra work, file a task and flip it in_progress.
- **Don't entangle your edits with an agent's commit.** If you write a
  file while an agent is mid-merge/commit in the same worktree, it can
  get swept into their commit. Sequence your edits for after they land,
  or write to /tmp and `cp` into place once the worktree is clean.
- **Cheap reads.** Prefer targeted source greps over re-`Read`ing whole
  files; you're juggling many threads' context.

## How real users actually use oxplow (surfaces to exercise)

Drive these like a user would, not just the agent terminal:

- **Comments are the human's voice.** Anchor a comment to a task/page
  (`create_comment`; intent `followup` = "agent, act on this" vs `note`
  = private). It surfaces in the **Comments Dashboard** and bumps the
  **"For me"** badge. A *different* thread's agent (even a different
  runtime, e.g. opencode) answers it via `list_comments` →
  `respond_to_comment` → `resolve_comment`. Exercise this cross-thread,
  cross-agent Q&A loop — it's a core collaboration primitive and it works.
- **Query/investigation threads + wiki.** Point a second thread at a
  question ("trace subsystem X end to end") and have it write a wiki
  page; verify it saved (`.oxplow/wiki/<slug>.md`) and that **⌘K BM25
  search** surfaces it across tasks/comments/wiki/files.
- **The claim-first effort model.** Agents claim the files they touch
  (`complete_task touched_files`; structured edits also auto-claim via
  the hook); the snapshot is the audit that reconciles claims
  (`changed_but_not_claimed` / `claimed_but_not_changed`). NOTE: a file
  YOU (navigator) or another writer touch in the worktree during an
  open effort gets attributed to that effort — expect attribution noise
  under concurrent/external writes.
- **Cross-stream / worktree parallelism + merges.** Run agents in
  parallel across streams (a primary stream + worktree streams) to get
  more done, then integrate to the primary. Merge via the **branch
  picker** on a stream's branch button: `⎇<branch>` switches,
  **`⟲<branch>` merges that branch into the current stream**
  (→ `git_merge_into`). Merges delegate to git: a dirty/active target
  worktree is refused, and conflicts leave `MERGE_HEAD` + a tracked
  `conflictedCount`. The regular-user way to resolve a conflict is to
  **hand it to an agent on that stream** ("resolve the in-progress
  merge, commit it"), then verify. Only merge into a CLEAN, idle target
  stream — never while its agent is mid-edit there.
- **Review surfaces.** Git Dashboard → commit pages (zones / treemap /
  co-change), the Tasks page, per-effort "Coverage & tests" +
  static-analysis + nudge panels are how a user understands what an
  agent did — read them, not only `git log`.

## Task discipline

- **Every bug/feature you find → an oxplow task** (UI: ⇧⌘N anywhere,
  or "Save and Another" for batches; or `create_task` IPC). Title =
  symptom; description = repro + root-cause notes from your own
  source reading + `## Acceptance criteria` including tests. Tasks
  you file are the driver's queue — write them so a fresh session
  can act without you.
- Your own direct work (infra fixes) also gets a task, flipped
  `in_progress` before editing and `done` after verification.
- Don't fix product bugs yourself — that's the driver's job and half
  the point is exercising the dispatch loop.

## QA/design notes (for the human, not the queue)

Maintain `ideas/qa/<date>-<topic>.md` with screenshots in
`ideas/qa/img/` — **never commit anything under ideas/qa/**. Keep
appending throughout the session (write via Bash heredoc — exempt from
the filing hook). Record:

- Bugs (with the task id filed) and evidence screenshots.
- Design/UX observations that aren't filable bugs: trust-breaking
  inconsistencies between surfaces, missing navigator affordances,
  friction in the pair-programming loop.
- **What works well** — credit specific affordances; the goal is a
  balanced review, not a defect list.
- Navigator-workflow meta-findings (where YOU had to babysit,
  guess, or work around the product — those are the highest-value
  product gaps).
- Corrections: when a deeper look changes a finding, append the
  correction rather than overstating — accuracy beats a long list.

## Wrap-up each session

- Settle every task you filed (driver finished it, or it stays
  `ready` with good context).
- Verify the QA notes file reads standalone — the user reads it cold.
- Report: what shipped (commits), what's queued, top product gaps.
