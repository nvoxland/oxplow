# Working in this repo — full contributor guide

The project root `CLAUDE.md` is a lean, always-on index: the essential
rules plus the subsystem routing table. This doc holds the full detail
those essentials point at — repo layout, test/lint policy, and the
complete task-filing discipline. Read it when you need the *why* or the
exact mechanics; the always-on gist lives in `CLAUDE.md`.

## `.context/` is the knowledge base

`.context/` is the project's durable knowledge base. Treat it as the
authoritative place for anything you'd otherwise stash in agent memory —
project decisions, system mechanics, gotchas, conventions, "why we did
it this way" notes.

1. **Read the relevant doc before touching its subsystem.** They're
   short on purpose — skipping them costs more than reading them.
2. **Update the relevant doc in the same commit as your change.** Docs
   that drift from code are worse than no docs.
3. **Capture new knowledge in `.context/`, not in memory.** If you
   discover a non-obvious decision, a recurring gotcha, an undocumented
   convention, or something you'd want to remember next session — write
   it into the matching doc.

`Read` the matching `.context/<name>.md` before touching a subsystem.

When you finish a change that alters how a subsystem works, **update
the matching `.context/` doc in the same commit**. Concrete triggers:

- Added a new table / store / migration → update `data-model.md`.
- Added a new MCP tool, hook, or Stop-hook branch → update `agent-model.md`.
- Added a new IPC method or event type → update `ipc-and-stores.md`.
- Added or repurposed a CSS variable → update `theming.md`.
- Added a new fs watcher or git operation → update `git-integration.md`.
- Changed how the editor pane handles models, menus, or decorations → update `editor-and-monaco.md`.

Docs reference source by **path only** (no line numbers — they drift).

Use plan mode for multi-subsystem work (3+ areas touched) or ambiguous
requirements. Skip it for single-file changes, typos, renames, or narrow
refactors — go straight to TDD or a subagent dispatch.

## Filing enforcement (full)

**No trivial-edit carve-out for filing.** Every Edit / Write /
MultiEdit / NotebookEdit on project files requires a tracked work
item — typos, single-line CSS tweaks, and one-file fixes included.
Enforcement is a **PreToolUse hook**: when the writer thread has no
`in_progress` item AND no filing call has fired this turn, the edit
tool is denied at the moment it's invoked, not at end-of-turn. File
the item (or flip a ready row to in_progress), then re-issue the edit.
Bash is intentionally exempt — `git merge`, `git pull`, codegen, and
formatters mutate the worktree as a side effect without representing
authored change worth filing. Edits made while the worktree is mid
git operation (merge / rebase / cherry-pick / revert — i.e. when
`MERGE_HEAD` / `REBASE_HEAD` / `CHERRY_PICK_HEAD` / `REVERT_HEAD`
exists in the gitdir) are also exempt: the authored change is the
merge commit itself, and conflict resolution would otherwise dead-
lock against the filing rule. The `.context/` read rule still gets a
soft pass for tiny mechanical edits — just don't skip the task.

**Asking the user a question.** When your reply ends with a real
clarifying question, A/B/C choice, or any ask where the user owns the
next move, call `mcp__oxplow__await_user({ threadId, question })` and
end your turn. The Stop hook honours this and suppresses every
directive (no dispatch nudge, no audit, no filing-enforcement) until
the user replies. Don't call it for rhetorical asides — only genuine
open questions.

## Repo layout (post-Tauri rewrite)

The backend is Rust; the desktop frontend is React/Monaco/xterm.

- `apps/desktop/` — the Tauri 2 desktop product. Frontend TS lives in
  `apps/desktop/src/`; the Tauri shell crate is at
  `apps/desktop/src-tauri/`. `tauri.conf.json` lives next to the
  shell crate; `bun run tauri:dev` (run from anywhere via root
  workspace scripts) boots Vite + the shell.
- `crates/` — reusable Rust libraries. `oxplow-domain` (pure types +
  store traits), `oxplow-db` (rusqlite stores + migrations),
  `oxplow-config`, `oxplow-fs-watch`, `oxplow-git`, `oxplow-session`,
  `oxplow-runtime` (write guard + filing enforcement),
  `oxplow-tmux`, `oxplow-pty`, `oxplow-lsp`, `oxplow-mcp`,
  `oxplow-coverage` (pure report-parse data types),
  `oxplow-collect-plugin` (pluggable collector registry + host
  parse helpers + jaq/Starlark/exec transform runtimes),
  `oxplow-app` (Services orchestration + shared boot orchestration in
  `boot.rs`), `oxplow-rpc` (transport-neutral command cores + the
  `rpc_dispatch!` registry; no tauri deps), `oxplow-daemon` (headless
  HTTP backend for remote dev — serves the dispatch over loopback,
  paired with an `ssh -L` tunnel), `oxplow-tauri-ipc`
  (`#[tauri::command]` adapters + `tauri-specta` exports; one-line
  delegates into `oxplow-rpc`).
- Old top-level `src/` (the Electron/Node backend) is gone; nothing
  TS lives at the repo root anymore.

## Tests

Each crate has its own `cargo test` suite. Cross-crate behavior tests
live in `crates/oxplow-app/`. Don't mock the DB — tests use
`oxplow_db::Database::in_memory()` (a fresh in-memory SQLite per
test) or a tempfile-backed DB.

Frontend tests still use `bun test` (run from `apps/desktop/`); root
`bun run test` invokes both Rust and TS suites.

**Closing a task → run `bun run test:collect`, not bare `cargo test` /
`bun test`.** `test:collect` (`cargo cov && bun run --cwd apps/desktop
test:junit`) is the configured `collection.testCommand` — it's the only
test run that emits the JUnit + lcov reports oxplow parses into the
effort's "Coverage & tests" panel.

**For the red/green loop, use `bun run test:fast` (the configured
`collection.fastTestCommand`) rather than a bare `cargo test`.** It takes
the same filters (`bun run test:fast -p oxplow-git symlink`) and still
writes `target/nextest/default/junit.xml`, so the red→green progression
lands in the panel — it just skips coverage instrumentation, which is the
slow part (~11s for the full suite vs milliseconds for one filtered test).
`bun run test:fast:ts` is the frontend counterpart. Bare `cargo test` /
`bun test` emit **no reports**, so the run shows only the command and none
of the tests. The
Rust half needs `cargo-llvm-cov` + `cargo-nextest` installed (`cargo
install cargo-llvm-cov cargo-nextest`) to write `target/coverage/lcov.info`.
See `.context/collection.md`.

### Coverage floors & pass-through crates

Two CI gates enforce line coverage: a **workspace floor**
(`cargo llvm-cov --workspace --fail-under-lines N` in
`.github/workflows/ci.yml`) and **per-crate floors**
(`scripts/coverage-floors.py`, `FLOORS` dict). Floors sit a few points
below current measured coverage — a real regression fails CI, normal
churn doesn't.

**Don't coverage-chase the pass-through adapter crates**
(`oxplow-tauri-ipc`, `oxplow-mcp`). Most of their commands are one-line
delegates to `oxplow-rpc` cores (the bodies live there so the headless
daemon can share them — see `.context/remote-daemon.md`). Those
delegates are *typed*, so a mis-wire usually fails to compile; a
"call it, assert it didn't panic" test adds a coverage point and ~zero
bug-catching. Their per-crate floor is a **catastrophe-catcher** (it
trips if the adapters regress toward 0% / the test harness breaks), not
a line-coverage target — keep it well below measured and don't pad to
raise it.

What *is* worth testing in `oxplow-tauri-ipc`: the genuinely Tauri-only
files that can't live in `oxplow-rpc` (`menu.rs` accelerator/separator
parsing, `launch.rs` fs checks, `webview.rs`, `state.rs` `LaunchInfo`
wire contract) and any adapter that *computes* something locally
(e.g. `list_recent_projects`' exists-flag). The real safety net for the
command surface is the `export_ts_bindings` test + the
`oxplow-surface-parity` crate (they catch a command missing from the
renderer surface, or the IPC/daemon transports drifting) — not line %.

## Rust formatting & lints

CI runs `cargo fmt --all -- --check` AND `cargo clippy --workspace
--all-targets -- -D warnings`. Whenever you edit a `.rs` file, before
ending the turn:

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets -- -D warnings` and fix
   anything it surfaces. Treat warnings as errors here — that's how
   CI runs. Don't sprinkle `#[allow(...)]` to silence a real lint;
   only use it when the lint genuinely doesn't apply (e.g. a public
   API that intentionally has many args).

Both checks have no functional test signal — they're purely
formatting/lint hygiene, and drift accumulates silently between
commits. The durable fix is a PostToolUse hook on `Edit`/`Write` that
runs `rustfmt` + `cargo clippy --fix` against the touched crate.
Until that's installed, run both manually each turn.

## Sources must stay searchable

CI runs `bun run lint:searchable` (`scripts/check-control-chars.py`),
which fails on a NUL byte in any tracked non-binary file.

This is a correctness check, not style. A NUL makes git classify the
file as **binary**, and grep / ugrep / ripgrep then refuse to print
matches — while the file still compiles and its tests still pass. The
file becomes invisible to every codebase search, and the search reports
success with zero hits rather than an error. In a repo an agent
navigates by search, that is the worst possible failure mode: silent,
asymmetric, and indistinguishable from "no such symbol".

It has already happened three times, all the same way — a raw control
character used as a key separator in a template literal
(`` `${a}<NUL>${b}` ``) instead of the `\u0000` escape. Write the escape.

If you add a genuinely binary file type, add its extension to
`BINARY_EXTS` in the script rather than weakening the check.

## Tasks are observational

Oxplow passively tracks active agent turns: each open `agent_turn` row
(`ended_at IS NULL` and started after runtime boot) renders as a live
row in the Work panel's in_progress bucket showing the prompt and a
spinner. When the turn Stops, the row disappears. No synthesized work
items, no auto-file/auto-complete, no adoption — you don't need to
narrate turn boundaries.

**File a durable task before you start editing** (unless the
change qualifies for the trivial-edit carve-out above). When you're
about to change project files in a turn and you aren't already working
against an existing item, file one with status `in_progress`. The item
should describe the real piece of work you're committing to ship, not
a placeholder. When it's settled, call `complete_task` to ship an
explicit summary.

**Pick `create_task` (kind defaults to `task`) for one coherent
change**, even if it spans a few files. Use `file_epic_with_children`
only when the work has ≥3 sub-steps a reviewer would naturally
inspect independently — distinct phases, handoffs, or separable
subsystems. The test: could a child close to `done` on its own
and have the user inspect just that piece? If no, it's one task.

**One user-visible concern per ROW.** Independent concerns must be
separate items (sibling tasks or epic children) — a reviewer has to be
able to accept one and push back on another. Test: if a reasonable
reviewer would want to check them independently, they're separate
rows.

**Multiple independent asks in a single prompt → multiple tasks.**
When the user packs two or more independent things into one message
(e.g. "fix X. ALSO: do Y", "do A, then B"), file a separate task
for each one before starting work — even if you'll do them in the same
turn. They are independent concerns by definition; a reviewer must be
able to accept one and reject the other. Don't lump them under a
single "do everything the user asked" task.

**Every new ask gets its own item.** When the user sends a new request
mid-turn, file a new task rather than silently expanding the
current item's scope. The exception: if the new ask is genuinely a
correction to the same concern (a fix/redo on something you just
shipped to `done`), reopen that item — call `update_task`
to flip it back to `in_progress`, redo the work, then `complete_task`
back to `done`. Filing a "Fix what I just did" task
fragments the history.

**Mid-turn user prompts are a new ask boundary.** When a
`<system-reminder>` injects a new user message while you are still
working on something, treat it as a fresh ask — not as more scope for
the current `in_progress` item. Default action: file a new row before
the next edit. Only stay inside the current item if the new prompt is
a direct correction to that exact item; otherwise the rule above
applies. The Work panel must reflect every distinct concern the user
raised, not just the first one. Runtime nudge: a UserPromptSubmit
reminder fires whenever a new prompt arrives and the thread already
has an `in_progress` item from a prior prompt — it points at the open
item and asks you to choose explicitly. Don't ignore it.

**File backlog ideas as you have them.** When you notice a follow-up
worth doing later — a deferred polish item, a TODO surfaced while
finishing something else — file it as a `ready` task right then.
Don't bury follow-ups in prose at the end of a reply where they'll be
forgotten. The backlog is the durable record; replies are not.

The runtime handles the rest of the state machine for you: tasks
persist across turn boundaries automatically, the Stop hook reminds
you to audit `in_progress` items only when something actually changed,
and the redo-detection hint on `create_task` flags when a new
"Fix …" task probably belongs as a reopen instead.
