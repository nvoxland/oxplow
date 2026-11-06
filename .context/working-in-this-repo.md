# Working in this repo — full contributor guide

The project root `AGENTS.md` is a lean, always-on index: the essential
rules plus the subsystem routing table (`CLAUDE.md` only imports it, for
Claude Code, which doesn't read `AGENTS.md` itself). This doc holds the full detail
those essentials point at — repo layout, test/lint policy, and how
work is tracked. Read it when you need the *why* or the
exact mechanics; the always-on gist lives in `AGENTS.md`.

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

### Agent coordination

Main Controller owns the remaining extension foundations. At the owner's
request, independent Bugfixes worktree assignments and mailbox coordination
have ended. Main delegates bounded work to managed subagents on lighter models;
they share Main's workspace and receive explicit disjoint file scopes. Main
owns contracts, shared wiring, security review and final integration. Keep
durable plans in `.context/`; the current sequence and ownership are in
[foundation-execution-plan.md](./foundation-execution-plan.md).

The retired append-only mailbox remains historical evidence. Its receipt bug
was a read/append race: advancing to the latest Main message could skip a peer
reply posted in between. If such a mailbox is reused, track and acknowledge
peer message UUIDs individually. A file does not wake an idle session.

### oxplow-dev: tasks and the wiki while the work list is none

The work list, effort policy, snapshots and wiki are `oxplow-foundation`'s
defaults, so disabling `oxplow-bundled` (`disableExtensions: [oxplow-bundled]`
in `.oxplow/personal.yaml`, never committed) no longer removes them. A
project that chose no work list (Settings → Capabilities) has none:
`work_item.*` is refused and the task tools are hidden. The
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
cargo run -q -p oxplow-dev -- wiki list
cargo run -q -p oxplow-dev -- wiki show some-page
cargo run -q -p oxplow-dev -- wiki write some-page [--title …] [--file body.md]   # body from stdin without --file
cargo run -q -p oxplow-dev -- wiki delete some-page
cargo run -q -p oxplow-dev -- --thread thr1 wiki note "…"
cargo run -q -p oxplow-dev -- wiki search burrows
cargo run -q -p oxplow-dev -- extension status
cargo run -q -p oxplow-dev -- extension deploy                     # everything on whose source changed
cargo run -q -p oxplow-dev -- extension deploy oxplow-foundation   # one extension
cargo run -q -p oxplow-dev -- --help
```

`extension status` and `extension deploy` change what oxplow runs from
outside it. Boot deploys nothing, so they are how a checkout gets its
first deployments and how oxplow comes back when it can't fix itself:
oxplow-foundation not running (no agents, no Alerts, no Settings, no core
commands; the shell's "oxplow-foundation isn't running" dialog names the
command), usually a deployment an older oxplow recorded that doesn't
activate against the core you just built. `status` is read-only and safe
while oxplow runs: it activates the active deployments in a throwaway
oxplow in memory, as the next boot would (`extension_deploy::next_boot`),
and says what each runs, whether its source changed since, and what would
be held back and why. `deploy <name>` deploys that source as the app does
(built, copied into the store, activated, recorded active), confirming its
permissions and build as the person running it; `deploy` with no name does
that for every extension that is on and whose source isn't what runs,
oxplow's required ones first. There, one that isn't required and doesn't
load is held back, as boot holds it: the deploy prints why and still
succeeds, so `run:dev` starts oxplow (a required one failing, or the one
named, fails it). It refuses while oxplow holds the project
(the instance lock: a running daemon keeps the live set in memory and its
next publication would undo it), so quit oxplow, deploy, then start it.
With foundation running, deploy from inside oxplow instead
(`oxplow.extension.deploy`, or Deploy in the Extensions page): it
replaces what runs at once, no restart. Nothing deploys on its own: a
source may be mid-way on purpose, so a person or their agent decides.

**Recovering a broken oxplow** (it boots without foundation, or an
extension is held): run `extension status` to see what runs and why
something is held; fix the source if the problem is in it; quit oxplow;
`extension deploy` (or `extension deploy <name>`); start oxplow. To go
back to an earlier deployment instead, use `oxplow.extension.rollback`
from inside oxplow (it needs foundation running).

`wiki` runs the `oxplow.knowledge.*` commands on the same bus with oxplow's
wiki as the store, whatever the project has active (pages are
`.oxplow/wiki/<slug>.md`, recorded as the app records them, links checked).
`list`, `show` and `search` read the recorded `wiki_page` rows (`search` is a
case-insensitive substring over slug, title and body, not the site search
index). `wiki note` needs a thread (`--thread` or `$OXPLOW_THREAD_ID`).
`wiki delete` is confirmed by being named.

`--help` / `-h` / `help` print this and do nothing, before the database
is opened; an option a command doesn't take is refused (both used to
become a task's title — tasks named `--help`).

It runs the task system's own `work_item.*` commands on a command bus
over the project's database (validation, audit and events as the app
writes them; the acting thread is `$OXPLOW_THREAD_ID` or `--thread`), on
a bus that checks nothing active. So it's the same tasks, and
re-enabling the extension shows everything done meanwhile. The database
is opened as it is (`Database::open_existing`: no migrating, no dataset
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
  shell crate; `bun run build:dev` (root workspace script)
  builds the frontend and the shell; run `./target/debug/oxplow .`.
- `extensions/oxplow-foundation`, `extensions/oxplow-bundled` — oxplow's
  own extensions, compiled into a release build by
  `crates/oxplow-app/build.rs` (every file in the folder ships; a `build:`
  runs there first). A debug build and the Rust tests read them from the
  folder at run time instead, so editing one never recompiles a crate.
  Building oxplow needs bun, as its extension builds use it.
- `crates/` — reusable Rust libraries. `oxplow-domain` (pure types +
  store traits), `oxplow-db` (rusqlite stores + migrations),
  `oxplow-tasks` (oxplow's own task list, one implementation of the
  work-item interface: its store, service and status mapping; nothing
  outside it names them — `.context/work-items.md`),
  `oxplow-config`, `oxplow-fs-watch`, `oxplow-git`, `oxplow-session`,
  `oxplow-runtime` (the write guard),
  `oxplow-pty`, `oxplow-lsp`, `oxplow-mcp`,
  `oxplow-harnesses` (the built-in agent harnesses — Claude Code, Codex,
  opencode — each launching its sessions and writing its runtime;
  depends only on the domain), `oxplow-agent-text` (core's skills,
  slash commands and answerability questions),
  `oxplow-coverage` (pure report-parse data types),
  `oxplow-script` (the jaq/Starlark/exec script runtimes collectors,
  effects and command handlers run in, + host parse helpers),
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
  SQLite has `log2`, `pow`, … (SQL datasets use them). A build that
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
crate's own graph does: the staged sidecars (oxplow-desktop's build
script validates them, so a workspace test or clippy run fails without
them) and the fake provider's binary (oxplow-app's provider tests spawn
it). Both used to fail a new worktree's first run. Both are built every
run (a no-op when current) and the sidecars restaged when they changed,
because tauri-build copies a staged sidecar over `target/debug/<name>`:
a stale one ran old script-worker code under the tests.

**A pure crate's tests start in seconds.** `test:fast` runs through
`scripts/test-fast.sh`: when every `-p` names a crate whose graph (dev
dependencies too) never reaches oxplow-script, oxplow-app or
oxplow-provider-fake (its `PURE` list: oxplow-config, oxplow-domain,
oxplow-db, oxplow-provider-protocol, …), it skips `test-prereqs.sh`
(only the target sweep runs), so `bun run test:fast -p oxplow-config
metric_expr` takes ~4 s instead of rebuilding the sidecars first. A
crate that gains a dependency on one of those three leaves the list.

**Running oxplow from source: `bun run run:dev`** (`scripts/run-dev.sh`),
with oxplow quit: `bun run build:dev`, then `oxplow-dev extension deploy`
(what changed, foundation first), then `./target/debug/oxplow .`; it
refuses before building while oxplow holds the project. Never a `cargo
build` of the shell alone. There is no Vite dev server or `tauri dev`
route. Extension changes redeploy in the running app. The shell runs the daemon beside it, and
building the shell copies the *staged* daemon there, so without staging
first it runs the last staged one, however old; `DEV.md` "Run from
source" has the rest.

**Which tests and lint to run, and when, is the agent's call** — run
what the change warrants; nothing requires a full run before a commit.
Use the report-emitting commands below, not bare `cargo test` /
`bun test`. `test:collect` (`scripts/test-collect.sh`: `cargo cov`,
then `bun run --cwd apps/desktop test:junit` whether or not the Rust
suite passed, failing if either did) is the configured `testing.command` — it's the only
test run that emits the JUnit + lcov reports oxplow parses into the
effort's "Coverage & tests" panel.

**Which check for which change:**

| You changed | Run |
|---|---|
| Only files under `extensions/` or `.oxplow/extensions/` | `bun run test:ext [<name>]` — seconds, no compile; `bun run test:ext <name> --only <fixture>[,…]` runs the check and just those fixtures while you iterate |
| A Rust crate | `bun run test:fast -p <crate> [filter]` |
| `apps/desktop/src` | `bun run test:fast:ts` (and `bun run e2e` for a user path) |
| Core and an extension together | both: `test:ext` rebuilds through `cargo run` first (and builds `oxplow-script-worker` first: a fixture's scripts run in it, from `target/debug/`) |
| Ready to push | `test:collect` with `lint:collect`, once |

An extension's behavior is a fixture in its extension, written first
(`extensions-sdk.md` "`oxplow extension test`"): a page, a command, an
event's effects, a job, a collector, an advisory, a query over its
datasets, a custom component's own code (`ui:` steps, "Testing a
component"). Never a new Rust test: Rust tests are for core. A
fixture runs on a throwaway oxplow — this build's schema, only the data
it makes, only the extensions its own declares (a fixture's `requires:`
adds more, e.g. `oxplow-bundled` for oxplow's work list) — never the
project's database. `ui:` steps need the app built once (`bun run build:dev`).
**Every script run needs `target/debug/oxplow-script-worker`** (beside the
executable that runs it). `cargo run -p oxplow-dev` builds only
`oxplow-dev`, so `test:ext` builds the worker first; `build:dev` and the
test and lint runs stage it with the other sidecars. A cleaned or swept
`target/` (`scripts/sweep-target.sh`, `cargo clean`) drops it from under a
running oxplow, which then disables every collector ("couldn't start the
script worker"): run `bun run build:dev`, then re-enable them.
`test:ext` writes `target/extension-tests/junit.xml`, which the project
extension's `tests.extension_junit` collector reads.

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
or dataset needs nothing: a rebuilt test binary is a new template key.

**Fixtures run the production activation.** `Services::in_memory`,
`in_memory_on_machine` and the fixtures built on them (`test_fixtures::services_with_effort`, …) are
async, return `Arc<Services>`, and end with `extension_activation.initialize`
exactly as boot does; there is no test-only way to register commands, datasets or
providers. A test that writes extension files or changes config after setup
is standing in for the generation owner: it calls `test_fixtures::activate(&svc)`
(asserts no extension was held back) or `activate_with_problems(&svc)` (returns
what was) at that point. Declared entities need no seeding: activation creates
them empty. (`in_memory_inactive` is crate-private, for a test of what happens
before the first generation.)

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
project — the real app, a real daemon, the fake terminal harness and the fake
work-item provider; `tests-e2e/README.md` has the harness. It isn't part
of `test:collect` (it builds the frontend and boots a daemon per spec,
~40 s); CI runs it as its own `e2e` job. **Run it when a change touches
what a person sees or does in the app** — a page, a rail section, a
command's UI path, a testid — and add or extend a spec under
`tests-e2e/specs/<area>/` for a new user path. What every page does
(select, expand, switch, forms, slots, replacements, embeds, components,
panels) is `tests-e2e/specs/pages/*`'s, each over a small extension of its
own: a page or component that moves into an extension gets fixtures for
its data and behavior (`ui:` steps for a component's own code) and a spec
of its own only for what only the app's shell does. Rules a spec keeps:

- select by `data-testid` (a renamed testid breaks specs — grep
  `tests-e2e/` first);
- wait on the page with web-first `expect`, and on the daemon with the
  helpers (`until`, `settle`, `searchable`, `waitForModels`) — never a
  sleep: global setup refuses a spec that calls `waitForTimeout`;
- seed through `run()` (the bus, as the person) or files written before
  boot, never through a backdoor; deploy and approve as a person does
  (`oxplow.extension.deploy`, confirmed inline);
- a spec whose state no other may touch first (nothing deployed, a
  provider configured) takes the `fresh` fixture, its own daemon.
- every thread runs the fake terminal harness (the test extension's
  `scripted` provider, `agents: [scripted]`); a spec that needs an agent
  whose process ends enables Claude, which the daemon's PATH (the person's
  own bin dirs left out) can't find on any machine, so its launch shell
  exits at once. (Codex, with no program to find, fails its launch before a
  PTY exists; the pane says why with a Retry — its own spec.)

A bug the suite finds is its own task, fixed with a unit test where one
can pin it; the spec stays as the end-to-end check.

### A test that acts on the whole process

CI's `cargo test` runs the tests of one crate as threads of one process;
`test:fast` (nextest) gives each test its own process. A test that calls
something process-wide therefore passes locally and breaks other tests in CI.
`process_group::stop_all` is the example: it ends every program group of the
process, so a collector, provider or build of another test dies with it. Run
such a test again, alone, in its own process
(`stop_all_reaps_every_live_group` starts `current_exe()` with `--exact`). To
check a suspect, run the test binary itself with `--test-threads=4`.

### Dependencies are optimized in dev and test builds

`[profile.dev.package."*"]` sets `opt-level = 1`. Unoptimized, parsing YAML,
hashing and regexes took most of a simple test's time (`oxplow-app` in one
process: 826 s to 495 s; a full local `test:fast`: 906 s to 339 s, under
different load). A clean build pays for it once. The workspace's own crates stay
at 0, so their rebuilds and backtraces don't change.

So don't tune a test script to the interpreter's speed. A "runaway" script loops
without allocating and can't finish: `n += i` over a large range hits the memory
cap first, and a loop sized to outlast a sleep ends early on a faster build.

### What CI runs, and why

A push runs `test`, `e2e`, `daemon-contract`, `audit` and `platforms` (`cargo
check` on macOS and Windows). `coverage` and `package` take hours and no release
waits for them (`scripts/ci-passed.sh` waits for `test` and `e2e`), so they run
at night and by `workflow_dispatch`. A coverage floor or an installer that breaks
shows up the next morning.

- Each Linux job shares one Cargo cache and only `test` saves it. When every job
  saved, the first to finish won, and one restored inside its own run had files
  newer than the checkout. Cargo then skipped `build.rs`, the gitignored
  `dist/` was never built, and hundreds of tests failed with
  `component.bundle_unreadable`. `rust-cache` removes the workspace's own build
  output, so this can't recur. If that error shows up on a fresh checkout with an
  old `target/`, remove `target/debug/build/oxplow-app-*`.
- A release reads the cache of the nightly `package` job. A cache saved on a tag
  is seen only by that tag, so a release saves none.
- `.github/actions/linux-tauri-deps` caches the downloaded packages: the Ubuntu
  mirror has taken 26 minutes to send these 59 MB.
- `[profile.release]` is thin LTO, not fat. On the daemon: relink 414 s to 195 s,
  clean build 819 s to 594 s, 80 MB to 91 MB, and 31.1 MB to 32.9 MB compressed
  (about 6%). Fat LTO is nearly all of a release's compile time, and it runs once
  for each of the four binaries.

### Timing assertions flake under `cargo cov` (tsk175)

`cargo cov` runs every test as its **own process, all concurrently, on an
instrumented binary** — so a test is 2-5x slower *and* competing with one
sibling per core. Any assertion on elapsed wall-clock can lose whole
scheduler quanta there while passing every time under `cargo test`, which
threads one process. That asymmetry is why such a failure shows up once in
a full run and then refuses to reproduce in isolation.

Prefer asserting **completion under a budget** over a wall-clock number
(`lcov_parser_parses_a_whole_workspace_report_without_timing_out` is the
model), and make the budget generous: a wait that returns as soon as the
event arrives costs nothing extra on a pass, while a 3 s budget for an
FSEvents delivery or a loopback redirect failed in most full runs on a
loaded machine (`a_top_level_dir_created_later_is_followed`,
`an_idle_connection_doesnt_hold_up_the_redirect` now allow 20–30 s). When
the deadline is the thing under test, retry the scenario on a lost race
(`a_listener_stops_when_its_time_is_up_with_nobody_waiting`). When you genuinely need a timing *ratio* — a curve-shape guard —
min-of-N sampling is **not** enough on its own, because all N samples of a
size can be descheduled together. Retry the whole comparison and pass on
any clean attempt (`first_ratio_under` in `oxplow-script`): noise
only ever ADDS time, so a spike ruins one attempt while a real regression
ruins every one — the guard stays exactly as strict, the false positives go
to ~0.

Waiting for a row to appear is not waiting for its work to finish: an
attempt's `effect_run` row exists from the moment it starts, so
`the_retry_loop_sends_a_due_retry` first asserted on a `started` row about
one run in five under load. Wait for the terminal state, then assert on it.

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

### Frontend module mocks are process-wide

Bun's `mock.module` replaces a module for the whole `bun test` process,
and the last mock wins. A file that mocks `api.js` as
`{ ...await import("../api.js"), … }` spreads **whichever mock ran
before it**, not the real module — another file's `listPersonCommands`
or `listExtensions` included. So it passes alone and fails in the full
run. A file that depends on a read's answer pins that read itself
(`PageTree.replaceable.test.tsx` mocks `api.js` for the reads it makes). Reproduce one of these by running the two files together:
`bun test <the earlier mocker> <the failing file>`.

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

## Async futures and the thread stack

Every thread runs on tokio's default 2 MiB stack — the daemon, the app and
the tests; there is no override. A command, effect, collector or model call
nests the others, and a debug build keeps each nested future inline in its
parent, plus a copy in every generic wrapper that takes it by value. So a
wrapper around a whole run (`ExtensionActivation::read`, `read_pinned`,
`SqlGateway::read_in` / `read_generation`) boxes its future first and returns
a small one: the nest below it holds a pointer, not the run. A new generic
wrapper over a command, AI or hook run does the same; a stack overflow in a
test is this, not a reason to raise the stack.

To measure: `RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR=<scratch> cargo rustc -p
oxplow-app --lib -- -Zprint-type-sizes`, filtered to `{async fn body …}` and
`{async block …}` (the largest were 88 KB before the wrappers boxed, 16 KB
after). To find the least stack a test needs, run it with
`RUST_MIN_STACK=<bytes>`. `collector_runner`'s default-stack test runs a
model-calling collector through the command bus on a 2 MiB thread.

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

3. **`pub(crate)` by default.** A module is `pub` only when another
   crate imports it. A `pub(crate)` module shows its dead code to the
   compiler, so unused items surface at once; a `pub` one hides them.
   `oxplow-app` groups the extension machinery in `src/extension/`
   (`mod.rs` lists which modules the other crates reach), and
   `source_guards.rs` pins the count of `pub mod` in its `lib.rs`: it only
   goes down. Page and viz spec types live in `pages/spec.rs`; page files
   never take them from `extension/extensions.rs`, the loader.

Both checks have no functional test signal — they're purely
formatting/lint hygiene, and drift accumulates silently between
commits. The durable fix is a PostToolUse hook on `Edit`/`Write` that
runs `rustfmt` + `cargo clippy --fix` against the touched crate.
Until that's installed, run both manually each turn.

## Builds and `target/` (tsk881)

**One feature set per third-party crate, for every build.** Cargo's
resolver picks a dependency's features from what the packages being
built ask for, so `cargo nextest -p oxplow-app`, `cargo build -p
oxplow-desktop`, clippy and a workspace build each
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
  (`oxplow-domain`, `oxplow-script`, `oxplow-code-dup`,
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
- **What still accumulates** (a new hash per real dependency change and
  a fresh incremental cache beside the old one — cargo deletes nothing)
  is swept automatically: `scripts/sweep-target.sh` (cargo-sweep, `cargo
  install cargo-sweep --locked`) drops artifacts of toolchains no longer
  installed, then the oldest until `target/` is under 30 GB. The test and
  lint runs call it first (`test-prereqs.sh`), at most every 30 minutes;
  `bun run clean:target` runs it now. 30 GB sits above one worktree's
  working set (a full test + clippy + binary build, ~23 GB measured), so
  what's in use stays. It orders by when an artifact was built, not last
  used, so a sweep that has to cut deep can drop a long-reused dependency
  and cost one rebuild of it. Without it one worktree reached 59 GB in a day —
  13 incremental caches of oxplow-app, 8 of them dead since the
  morning's merges — and three worktrees filled the disk. Coverage builds
  live apart in `target/llvm-cov-target` (`cargo cov`).
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
[extensions-sdk.md](./extensions-sdk.md) "The SDK"). It costs a real run, so run
it when the skill or the SDK changes enough that the recording no longer
says something true — not per commit. Commit what it writes (`run.json`,
`produced/`, `check.txt`, `test.txt`) with a hand-written `notes.md`;
`recorded_agent_runs_still_check_and_test_clean` replays `produced/`, and
a `prompt.md` with nothing recorded fails it.

## What oxplow ships must work in another person's project

Oxplow is used on repos that are not oxplow's checkout. Three guards keep
this checkout out of what it ships:

- `the_agent_text_never_assumes_the_oxplow_checkout`
  (`crates/oxplow-agent-text/src/lib.rs`): no skill, reference, asset or
  slash command names `examples/extensions`, `extensions/oxplow-`,
  `crates/`, `tests-e2e`, `cargo `, `bun run`, `.context`, or a bare
  `oxplow extension …`. The CLI is `$OXPLOW_BIN extension …` (`oxplow` on
  PATH only once the person ran "Install `oxplow` command line…").
  `.oxplow/extensions/` is the project's own folder and stays legal.
- `shipped_text_never_names_the_checkout`
  (`crates/oxplow-app/src/bundled_extensions.rs`): the same for the
  shipped extensions' manifests, pages and scripts (not fixtures or
  `dist/`).
- `bun run lint:docs` (`scripts/check-docs-portable.py`, CI): `docs/` and
  `README.md` hold no dev-loop text (a web link may name a repo path).
  `DEV.md` "Run from source" is where building and running from source go.

The shell's File ▸ "Install `oxplow` command line…" (`install_cli`,
`crates/oxplow-tauri-ipc/src/commands/install_cli.rs`) links the running
app's binary as `oxplow` in `/usr/local/bin`, else `~/.local/bin` (with a
PATH hint). The foundation-down dialog shows only the backend's held
reason, never a dev command (`apps/desktop/src/foundationDown.ts`).

The generated `docs/reference/schemas/extension.schema.json` cites no
`.context/` doc either (a schemars doc comment is published text).

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

