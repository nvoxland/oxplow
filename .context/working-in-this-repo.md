# Working in this repo — full contributor guide

The project root `CLAUDE.md` is a lean, always-on index: the essential
rules plus the subsystem routing table. This doc holds the full detail
those essentials point at — repo layout, test/lint policy, and how
work is tracked. Read it when you need the *why* or the
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
- Profiled something, optimized a hot path, or **ruled an optimization out** →
  update `performance.md`. Negative results especially: "we measured X and it
  isn't worth doing" is the first knowledge lost and the most expensive to
  re-derive.
- Changed how the editor pane handles models, menus, or decorations → update `editor-and-monaco.md`.

Docs reference source by **path only** (no line numbers — they drift).

**No task ids in the repo.** Don't cite oxplow task ids (`tsk42`,
"epic tsk40") in docs, code or test comments, model descriptions,
migrations or commit messages. They live only in one project's local
database and mean nothing to anyone else. Say what the thing is or why
it's that way instead. Older ids still in the tree predate this rule;
drop one when you edit its line. Applied migrations are the exception:
never edit them (refinery checksums them).

Use plan mode for multi-subsystem work (3+ areas touched) or ambiguous
requirements. Skip it for single-file changes, typos, renames, or narrow
refactors — go straight to TDD.

## Tracking work

oxplow infers the work ([work-tracking.md](./work-tracking.md)): it opens
an effort when a turn changes files, links it to an item someone starts,
and closes it when a commit lands its work. Nothing is filed or closed
to be allowed to edit or stop, and an effort's files and test runs are
observed, never declared. A path in `generated.exclude`
(`.oxplow/project.yaml`) is build output oxplow deliberately doesn't
snapshot, so no effort owns it — e.g.
`apps/desktop/src/tauri-bridge/generated/bindings.ts`, which Tauri
Specta rewrites on nearly every build.

File a task when it helps the person follow the work: an epic and its
children for a plan of separately reviewable steps, one item per
independent ask they want tracked, and a `ready` item for a follow-up
you spot but won't do now (the backlog is the durable record; a reply
is not). A redo of something just shipped reopens its item rather than
filing "fix what I just did".

**Asking the user a question.** When your reply needs the user's
answer, end it with the question itself. A final message that ends in
a question shows the thread as waiting on them (`.context/work-tracking.md`
"No gates"), until their next prompt.

### oxplow-dev: tasks while the work list is none

Stripping oxplow to its core (disabling `oxplow-bundled` in
`.oxplow/project.yaml`, uncommitted) leaves this project with no work
list, so `work_item.*` is refused and the task tools are hidden. The
development helper `oxplow-dev` (`crates/oxplow-dev`, never shipped or
bundled) keeps oxplow's own tasks usable meanwhile:

```
cargo run -q -p oxplow-dev -- task list [--all]
cargo run -q -p oxplow-dev -- task show tsk12
cargo run -q -p oxplow-dev -- task create "Title" [--body …] [--parent tsk1] [--state in_progress]
cargo run -q -p oxplow-dev -- task transition tsk12 done
cargo run -q -p oxplow-dev -- task update tsk12 [--title …] [--body …] [--parent …]
cargo run -q -p oxplow-dev -- task comment tsk12 "…"
cargo run -q -p oxplow-dev -- task link tsk12 tsk13 blocks
cargo run -q -p oxplow-dev -- --help
```

`--help` / `-h` / `help` print this and do nothing, before the database
is opened; an option a command doesn't take is refused (both used to
become a task's title — tasks named `--help`).

It runs the task system's own `work_item.*` commands on a command bus
over the project's database (validation, audit and events as the app
writes them; the acting thread is `$OXPLOW_THREAD_ID` or `--thread`), on
a bus that checks nothing active. So it's the same tasks, and
re-enabling the extension shows everything done meanwhile. The database
is opened as it is (`Database::open_existing`: no migrating, no model
recompile under the running app; refused at another schema version).
The running app picks up the new events with its next pump pass; with
the work list none, no effort policy reacts to them. Its reads (`list`,
`show`) are oxplow's `task` table itself, not the work-item interface
(which shows only the active list's items) — the one reader of oxplow's
tables outside its implementation, on purpose.

## Repo layout (post-Tauri rewrite)

The backend is Rust; the desktop frontend is React/Monaco/xterm.

- `apps/desktop/` — the Tauri 2 desktop product. Frontend TS lives in
  `apps/desktop/src/`; the Tauri shell crate is at
  `apps/desktop/src-tauri/`. `tauri.conf.json` lives next to the
  shell crate; `bun run tauri:dev` (run from anywhere via root
  workspace scripts) boots Vite + the shell.
- `crates/` — reusable Rust libraries. `oxplow-domain` (pure types +
  store traits), `oxplow-db` (rusqlite stores + migrations),
  `oxplow-tasks` (oxplow's own task list, one implementation of the
  work-item interface: its store, service and status mapping; nothing
  outside it names them — `.context/work-items.md`),
  `oxplow-config`, `oxplow-fs-watch`, `oxplow-git`, `oxplow-session`,
  `oxplow-runtime` (the write guard),
  `oxplow-pty`, `oxplow-lsp`, `oxplow-mcp`,
  `oxplow-coverage` (pure report-parse data types),
  `oxplow-collect-plugin` (the bundled report parsers + host parse
  helpers + jaq/Starlark/exec transform runtimes),
  `oxplow-app` (Services orchestration + shared boot orchestration in
  `boot.rs`), `oxplow-rpc` (transport-neutral command cores + the
  `rpc_dispatch!` registry; no tauri deps), `oxplow-daemon` (headless
  HTTP backend for remote dev — serves the dispatch over loopback,
  paired with an `ssh -L` tunnel), `oxplow-daemon-sim` (the same
  daemon with its secrets in memory: the browser suite's, dev-only),
  `oxplow-tauri-ipc`
  (`#[tauri::command]` adapters + `tauri-specta` exports; one-line
  delegates into `oxplow-rpc`).
- Old top-level `src/` (the Electron/Node backend) is gone; nothing
  TS lives at the repo root anymore.
- **Build from inside the tree.** `.cargo/config.toml` sets
  `LIBSQLITE3_FLAGS = -DSQLITE_ENABLE_MATH_FUNCTIONS` so the bundled
  SQLite has `log2`, `pow`, … (SQL models use them). A build that
  doesn't read that file (a packager outside the tree) must set it
  itself: every `Database` open probes `log2` and refuses to start
  without it (`DbInitError::Build`, tsk726).

## Tests

Each crate has its own `cargo test` suite. Cross-crate behavior tests
live in `crates/oxplow-app/`. Don't mock the DB — tests use
`oxplow_db::Database::in_memory()` (a fresh in-memory SQLite per
test) or a tempfile-backed DB.

Frontend tests still use `bun test` (run from `apps/desktop/`); root
`bun run test` invokes both Rust and TS suites.

**A fresh worktree runs as-is.** `test:fast`, `test:collect` and
`lint:collect` start with `scripts/test-prereqs.sh`, which builds what no
crate's own graph does, only when it's missing: the staged sidecars
(oxplow-desktop's build script validates them, so a workspace test or
clippy run fails without them) and the fake provider's binary
(oxplow-app's provider tests spawn it). Both used to fail a new
worktree's first run.

**Which tests and lint to run, and when, is the agent's call** — run
what the change warrants; nothing requires a full run before a commit.
Use the report-emitting commands below, not bare `cargo test` /
`bun test`. `test:collect` (`scripts/test-collect.sh`: `cargo cov`,
then `bun run --cwd apps/desktop test:junit` whether or not the Rust
suite passed, failing if either did) is the configured `testing.command` — it's the only
test run that emits the JUnit + lcov reports oxplow parses into the
effort's "Coverage & tests" panel.

**For the red/green loop, use `bun run test:fast` (the configured
`testing.fastCommand`) rather than a bare `cargo test`.** It takes
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

**A full run can take lint alongside** — they don't share a target dir:
`(bun run lint:collect >/dev/null 2>&1 & bun run test:collect; wait)`,
still one foreground command (see [performance.md](./performance.md) →
"The dev loop"). CI treats clippy warnings as errors, so Rust changes
need `lint:collect` clean before they're pushed.

**A test database comes from a migrated template**
(`Database::in_memory()` restores `<temp>/oxplow-db-templates/<build+date>.sqlite`
instead of running every migration, ~330 ms → a few ms). A new migration
or model needs nothing: a rebuilt test binary is a new template key.

**An effort needs no task, and neither does its fixture.**
`test_fixtures::services_with_effort()` opens an effort linked to nothing
(through `oxplow.effort.open`, so its event is logged like a real one); a test
about oxplow's tasks takes `services_with_task_effort()` instead (an
in-progress task, the effort linked to it; it derefs to the plain
fixture, so helpers take either). Reach for the task one only when the
test reads or relies on the task.

### The browser suite (`bun run e2e`, P11)

`tests-e2e/` drives the built frontend in Chromium (and, for custom
components, WebKit) against `oxplow-daemon-sim` over a throwaway git
project — the real app, a real daemon, the fake ACP agent and the fake
work-item provider; `tests-e2e/README.md` has the harness. It isn't part
of `test:collect` (it builds the frontend and boots a daemon per worker,
~40 s); CI runs it as its own `e2e` job. **Run it when a change touches
what a person sees or does in the app** — a page, a rail section, a
command's UI path, a testid — and add or extend a spec under
`tests-e2e/specs/<area>/` for a new user path. Rules a spec keeps:

- select by `data-testid` (a renamed testid breaks specs — grep
  `tests-e2e/` first);
- wait on the page with web-first `expect`, and on the daemon with the
  helpers (`until`, `settle`, `searchable`, `waitForModels`) — never a
  sleep: global setup refuses a spec that calls `waitForTimeout`;
- seed through `run()` (the bus, as the person) or files written before
  boot, never through a backdoor; approve as a person does
  (`approveProgram`, `approveCollector`);
- a spec whose state no other may touch first (nothing approved, a
  provider configured) takes the `fresh` fixture, its own daemon.

A bug the suite finds is its own task, fixed with a unit test where one
can pin it; the spec stays as the end-to-end check.

### Timing assertions flake under `cargo cov` (tsk175)

`cargo cov` runs every test as its **own process, all concurrently, on an
instrumented binary** — so a test is 2-5x slower *and* competing with one
sibling per core. Any assertion on elapsed wall-clock can lose whole
scheduler quanta there while passing every time under `cargo test`, which
threads one process. That asymmetry is why such a failure shows up once in
a full run and then refuses to reproduce in isolation.

Prefer asserting **completion under a budget** over a wall-clock number
(`lcov_plugin_parses_a_whole_workspace_report_without_timing_out` is the
model), and make the budget generous: a wait that returns as soon as the
event arrives costs nothing extra on a pass, while a 3 s budget for an
FSEvents delivery or a loopback redirect failed in most full runs on a
loaded machine (`a_top_level_dir_created_later_is_followed`,
`an_idle_connection_doesnt_hold_up_the_redirect` now allow 20–30 s). When
the deadline is the thing under test, retry the scenario on a lost race
(`a_listener_stops_when_its_time_is_up_with_nobody_waiting`). When you genuinely need a timing *ratio* — a curve-shape guard —
min-of-N sampling is **not** enough on its own, because all N samples of a
size can be descheduled together. Retry the whole comparison and pass on
any clean attempt (`first_ratio_under` in `oxplow-collect-plugin`): noise
only ever ADDS time, so a spike ruins one attempt while a real regression
ruins every one — the guard stays exactly as strict, the false positives go
to ~0.

`cargo cov` passes nextest `--no-fail-fast` so a flake **names itself** the
first time instead of the run cancelling before it prints which test failed.
Don't remove that flag.

### Frontend timers: never sleep-then-assert (tsk342)

A React state change fired from a `setTimeout` (the slide-out strips'
180 ms pointer-leave grace, say) lands **outside `act`**, so React only
*schedules* the re-render. A test that sleeps past the timer and then reads
the DOM races that render and loses under `bun test` load — once, then never
in isolation. Rules, applied in `TerminalTabStrip.test.tsx`,
`Navigator.test.tsx` and `useSlideoutStrip.test.tsx`:

- A change that **should** happen: `await waitFor(() => …)` — it polls and
  flushes through `act`. Never `await sleep(); expect(…)`.
- A change that must **not** happen: sleep *inside* `act` (`await
  act(settle)`) so anything pending has flushed before you look.
- Absence is a boolean: `expect(queryByTestId("x") === null).toBe(true)`,
  inside `waitFor` too — there a failing `toBeNull()` serializes the tree
  on every poll, which under load ran a menu test past bun's 5 s timeout.
  `expect(el).toBeNull()` on a present element serializes the whole
  happy-dom tree into the JUnit message — that is how
  `apps/desktop/test-report.xml` once reached 1.8 GB.

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
parsing, `launch.rs` fs checks, `webview.rs`, `windows.rs` labels/registry
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

## Builds and `target/` (tsk881)

**One feature set per third-party crate, for every build.** Cargo's
resolver picks a dependency's features from what the packages being
built ask for, so `cargo nextest -p oxplow-app`, `cargo build -p
oxplow-desktop` (`tauri dev`), clippy and a workspace build each
resolved different sets (tokio with or without `test-util`, seven
serde_json variants…). Every switch rebuilt the graph under new hashes
and kept every copy — 85 GB of `target/debug` was about five builds of
duplicates, and rebuilds were slow for the same reason. The
**`oxplow-workspace-hack`** crate (managed by
[cargo-hakari](https://docs.rs/cargo-hakari), config
`.config/hakari.toml`) pins the union: every core crate depends on it,
so any subset builds the same artifacts, and an alternating build is a
no-op. Build-script (host) dependencies keep their own features
(`unify-target-host = "none"`), so the app never ships what only a build
script asked for.

What the hack leaves out, and why (tsk885):

- **The providers stay out.** `oxplow-provider-{fake,mcp}`,
  `oxplow-provider-protocol` and the crates they're built from
  (`oxplow-domain`, `oxplow-collect-plugin`, `oxplow-code-dup`,
  `oxplow-code-metrics`, `oxplow-coverage`) are traversal-excluded and
  don't depend on the hack: a provider ships as its own binary
  (built with `-p`) and is built from its
  own dependencies only. Their second copy inside a workspace build is
  small.
- **The desktop stack stays out.** Tauri and the crates only it pulls in
  (AppKit's bindings, `semver`, `phf_shared`) are final-excluded, so the
  headless daemon never builds Tauri, AppKit or WebKit.
  `scripts/check-headless-graph.sh` (in CI) fails if the daemon,
  `oxplow-rpc` or a provider reaches any of them.

What the hack still adds to every core crate, the daemon included, is
features of crates they already build, unified so builds don't churn:
tokio's `test-util` (test APIs only); reqwest's `system-proxy` and
rustls's `ring` provider, which Tauri's updater turns on (so the daemon
also follows the macOS system proxy, and builds `ring` beside
aws-lc-rs).

- **After changing a dependency** (adding one, its version or
  features), run `cargo hakari generate` (and `cargo hakari manage-deps`
  for a new crate). A new crate the providers are built from goes in
  `traversal-excludes`; a new crate only Tauri pulls in, in
  `final-excludes`. CI fails when the hack is stale (`cargo hakari
  generate --diff`). Install with `cargo install cargo-hakari --locked`.
- **A workspace crate never takes a feature only tests turn on**: hakari
  covers third-party crates only, so a `test-support`-style feature on
  one of ours splits every crate above it. A test double is its own
  dev-only crate (`oxplow-ai-fake`, `oxplow-provider-fake`,
  `oxplow-oauth-sim`).
- **What still accumulates** (a new hash per real dependency change, old
  incremental sessions): `bun run clean:target` (cargo-sweep, `cargo
  install cargo-sweep --locked`) drops artifacts of toolchains no longer
  installed, then the oldest artifacts until `target/` is under 60 GB.
  Coverage builds live apart in `target/llvm-cov-target` (`cargo cov`).
- **`split-debuginfo = "off"`.** A fresh workspace build is ~17 GB, its
  object files 1.8 GB (`debug = "line-tables-only"`, none for
  dependencies), but that isn't where `target/` grew. macOS's default
  `unpacked` keeps every codegen unit's `.rcgu.o` in `deps/` for the
  debugger, and each incremental rebuild writes them under a new
  session suffix without deleting the old: after one day of rebuilds the
  main worktree held 44 copies of each `oxplow_app` unit, 88 GB of
  object files in a 79 GB-on-disk `target/`. cargo-sweep can't help: the
  copies belong to one artifact. `off` (Linux's default already) links
  `oxplow-app`'s test lib as fast (7–8 s) and leaves none; `packed` also
  leaves none but runs `dsymutil` per link (+4 s, a 468 MB `.dSYM`). The
  cost of `off` on macOS: backtraces name functions without file:line
  (panic messages keep theirs). For a debugging session that needs them,
  `CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=packed`. Object files already
  leaked stay until a `cargo clean`.

## Recording a fresh agent (`scripts/record-just-works.sh`)

`scripts/record-just-works.sh <kind>` records a fresh `claude -p` agent
building an extension from `crates/oxplow-sdk/fixtures/just-works/<kind>/
prompt.md` with nothing but the oxplow-extension skill (P7.C6, see
[extensions.md](./extensions.md) "The SDK"). It costs a real run, so run
it when the skill or the SDK changes enough that the recording no longer
says something true — not per commit. Commit what it writes (`run.json`,
`produced/`, `check.txt`, `test.txt`) with a hand-written `notes.md`;
`recorded_agent_runs_still_check_and_test_clean` replays `produced/`, and
a `prompt.md` with nothing recorded fails it.

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

