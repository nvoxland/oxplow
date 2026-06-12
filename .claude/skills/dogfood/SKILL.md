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
  session shows claude's resume picker (answer it in the pane).
  Don't restart immediately after `/clear` — the resume id updates
  on the next hook event, so a restart in that window resurrects the
  cleared session.

## The navigator loop

1. **Brief the driver in its terminal pane.** Click the thread, fill
   `.xterm-helper-textarea`, press Enter — long text lands as a
   bracketed paste and needs a **second Enter** to submit. Write real
   briefs: scope, constraints, file-the-task-first, tests + fmt +
   clippy, commit per concern, update `.context/` docs.
2. **Wait without burning context.** The reliable done-signal is the
   hook stream, not the dot: poll
   `POST /ipc/list_hook_events {"threadId":...}` in a backgrounded
   Bash loop and exit when the newest event kind is `stop`. (Don't
   key off `list_agent_statuses` output shape without checking it.)
3. **Work concurrently while it runs.** Explore other surfaces, file
   bugs, test features (comments, search, dashboards), or brief a
   second thread. Useful IPC for inspection:
   `get_thread_work_state` (`{"threadId":"thr1"}`), `list_threads`,
   `create_task` (shape:
   `{"req":{"threadId":...,"input":{"title","description","parent_id":null,"status","priority","author":null}}}`).
4. **Review what landed** through oxplow itself: Git Dashboard →
   commit pages (zones/treemap/co-change), the Tasks page, effort
   reviews. Read the driver's final terminal summary.
5. **Verify live, never trust the claim.** Rebuild + restart the
   affected half, reload, and exercise the actual fix in the browser
   before considering it done.
6. **`/clear` the driver when the queue is settled** and the session
   is huge — all context lives in the task system, so re-brief the
   fresh session with task ids only ("work tskNN, tskMM in this
   order; read each description").
7. **If the driver looks stuck, suspect death, not thought.** A
   transient API error drops it to its prompt with the dot stuck on
   Working. Check the terminal tail; nudge it with one message naming
   where it left off (uncommitted files, failing check).

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
`ideas/qa/img/` — **never commit anything under ideas/qa/**. Record:

- Bugs (with the task id filed) and evidence screenshots.
- Design/UX observations that aren't filable bugs: trust-breaking
  inconsistencies between surfaces, missing navigator affordances,
  friction in the pair-programming loop.
- **What works well** — credit specific affordances; the goal is a
  balanced review, not a defect list.
- Navigator-workflow meta-findings (where YOU had to babysit,
  guess, or work around the product — those are the highest-value
  product gaps).

## Wrap-up each session

- Settle every task you filed (driver finished it, or it stays
  `ready` with good context).
- Verify the QA notes file reads standalone — the user reads it cold.
- Report: what shipped (commits), what's queued, top product gaps.
