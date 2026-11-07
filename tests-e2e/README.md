# The browser suite: the real UI in a real browser

`bun run e2e` drives the built React app in headless Chromium — and the
custom-component specs in WebKit too — against a real daemon: Playwright
(`@playwright/test`), no `tauri-driver`. The frontend
reaches the daemon the way a remote window does: the transport reads its base
and token from localStorage (`oxplow.remoteBase`, `oxplow.remoteToken`), and
CORS is permissive for exactly this (`.context/remote-daemon.md`).

## How a run is put together

- **`playwright.config.ts`** (repo root). Its `webServer` builds the frontend
  once (`vite build`) and serves it with `vite preview` on 127.0.0.1, on a port of the checkout's own (from its path; `E2E_PORT` overrides it, should two collide), so a
  spec never meets a stale `dist/`. Off CI, a server already on that port
  is used as it is (see "Running it locally"). Every test boots its own
  daemon, so tests run fully parallel, files included.
- **`support/global-setup.ts`** builds `oxplow-daemon-sim`,
  `oxplow-harness-fake` and `oxplow-provider-fake` and hands their paths to the
  workers (`OXPLOW_E2E_DAEMON`, `OXPLOW_E2E_HARNESS_FAKE`,
  `OXPLOW_E2E_PROVIDER_FAKE`). With `OXPLOW_E2E_BIN=target/debug` it builds
  nothing and runs the binaries already there.
- **`fixtures/extension/`** — the test extension `e2e`, copied into every
  project before boot: a fake work-item provider (`bin/provider`, written
  then, runs the fake this checkout built; its `provider.json` is checked
  in and kept equal to the fake's declarations by
  `the_suite_fixture_declares_what_the_fake_does`), an effect, a dataset, a
  page, and a resource. Private, since `providers` is still
  experimental. The effect comments on each created task; a title with
  `[fail]` makes it fail (Delivery, Retry) and one with `[delete]` makes it
  delete the task (a proposal). Nothing in it is approved until a spec
  approves it. A spec that needs a model's answer (triage) points a
  `typesafe` provider at a stand-in for Jev's API of its own, so nothing
  leaves the machine.
- **The github example** (`examples/extensions/github`) is copied in too,
  its `sync.sh` swapped for one that prints `fixtures/github-prs.json`
  (two pull requests) — the documented example, no GitHub.
- **`support/daemon.ts`** starts one daemon: `oxplow-daemon-sim` (the daemon
  with its secrets in memory — nothing reaches the keychain) over a throwaway
  git project, with its own `OXPLOW_HOME`, `HOME` (no rc file read, no
  shell history written), `SHELL=/bin/sh`, and no global or system git
  config. Its
  stderr and stdout (its log lines) are kept in `tests-e2e/.output/daemons/` and named in a failure to
  start; whatever fails before it listens, the process is killed and its
  project removed, and `stop()` returns for a daemon a signal already
  killed. `ipc()` calls
  `/ipc/<name>` as the person; `run()` runs a bus command, confirmed;
  `settle()` waits until boot's background tasks are done; `waitForModels()`
  opens `/events` first (the daemon subscribes before answering the
  upgrade), does a write, and resolves once the daemon says each named
  dataset changed — so a seeding write is never raced;
  `deploy()` deploys an extension from its source as the person,
  confirmed; `searchable()` waits until site
  search has indexed a write;
  `until()` polls any such background state, trying a check that throws
  again and naming its last error on timeout. `ipc()` on a reply that
  isn't JSON names the call, the HTTP status and what came back.
- **`support/fixtures.ts`** — `test` and `expect` for specs:
  - `daemon`, one per spec. Its pages open on the stream's seeded
    thread, which runs the project's default agent — the fake terminal
    harness (`agents: [terminal]`) — so the suite never starts a real agent
    CLI. That harness is the extension's `terminal` provider (`bin/terminal`,
    the fake provider in harness mode, its `harness.json` kept equal to the
    fake's declarations like `provider.json`; the fake harness's binary is
    on the daemon's PATH), enabled in the project's `project.yaml` and approved at setup as a person would, so
    no spec depends on another having approved it.
  - A workspace that fails while booting is stopped and removed; each
    fixture tears down in a `finally`.
  - `daemon` is a spec's own (booted for it, stopped after it), so no spec meets
    what another left behind: work items, streams, settings, git commits. It is settled before any page opens — boot's background tasks
    done and the extensions' datasets published — and carries the `stream`
    and `thread` its pages open on.
  - **Templates, not boots from nothing.** Booting a workspace from nothing
    (git project, store, the suite's extensions deployed, their datasets
    compiled) is the same every time, so a worker does it once per *slot*
    and keeps a copy of the whole dir (project, home, store) as the slot's
    template. A test's daemon starts over a fresh copy of a template, at the
    template's own path: the database and the store hold absolute paths
    (the stream's worktree, each extension's runtime folder), so a copy
    elsewhere would point at the template. A worker has two slots: while a
    test runs on one, the next test's daemon boots on the other, so only a
    worker's first test waits for a boot. Measured at 3 workers (CI's
    count): 76 s with one slot, 62 s with two, for 5% more CPU.
  - `storageState` points the page at that daemon (`connectedTo()` builds one
    for a context of a spec's own, e.g. with another token).
  - `page` fails any spec whose page threw.
- **`support/pages.ts`** — `installExtension` writes an extension of the spec's
  own (manifest, pages, component files) into the project and deploys it.
- **`support/ui.ts`** — a person's moves (`expandRailSection`,
  `openFromLauncher` — which waits for the query's own row before Enter).
- **`specs/<area>/*.spec.ts`** — the specs. Wait with web-first `expect`,
  `waitForModels` or `until`, never a sleep: global setup refuses a spec
  that names `waitForTimeout` or `setTimeout` (`test.setTimeout`, a spec's
  time limit, is fine). `specs/harness/` checks the helpers themselves.
- **Reports**: a JUnit report at `tests-e2e/.output/junit.xml`, for a
  failed spec its trace under `tests-e2e/.output/results`, and every
  daemon's log under `tests-e2e/.output/daemons` (all gitignored). oxplow
  reads the JUnit report after a local run (the project extension's
  `tests.e2e_junit` collector), so each spec's flips, mean and max time are
  in `v_test_case_stat`.
- **Projects**: `chromium` runs every spec; `webkit` runs
  `specs/components/` (a custom component's frame, the one place the two
  engines are checked apart — the macOS window is WebKit).
- **CI**: the `e2e` job in `.github/workflows/ci.yml` — Chromium and WebKit,
  three workers, the JUnit report uploaded always and traces and daemon
  logs on failure. `daemon-contract` stays browser-free. A failed test is
  tried once more, and only that retry is traced (tracing every test costs
  ~15% of the suite's CPU). A spec that passes only on its retry still fails
  the run (`failOnFlakyTests`): it is a failure to fix, not hide. The retry
  says which kind it is, a race (passed) or a break (failed again), and the
  JUnit report marks it `flakyFailure`.
- **Flake history**: `bun run e2e:flakes [runs=20]` reads the JUnit
  artifacts of the last runs on main (`gh`) and prints, per spec and
  project, how many runs it failed or flaked in, and per run the wall time,
  the sum of test times and the average number of busy workers.

## Running it locally

- `bun run e2e` is the whole suite: about 70 s on 14 cores (the default, half
  the cores, is the fastest worker count; 4 and 10 are slower).
- For a loop on a few specs, keep `bun run e2e:serve` running in a
  terminal: it builds the frontend, rebuilds it on every change (`vite build
  --watch`), and serves it on the checkout's port. `bun run e2e` then uses
  that server and skips its own build and start. Add
  `OXPLOW_E2E_BIN=target/debug` to skip the cargo build too, after a build
  of your own: one spec then takes about 3 s.
- `bun run e2e:flake <spec> [times=10]` runs one spec many times at once,
  the way a race fix is confirmed before it is pushed.

## What the specs cover

`shell/` boot and connecting with a token; `work/` Go To and launcher search; `knowledge/` the comments inbox and
an extension's resource; `code/` a commit's diff; `data/` extension pages; `settings/`
Programs approval and the settings pages; `integrations/` the fake tracker
(configure, Check, Enable, active, Sync Now), Delivery (a failed reaction,
a person's Retry, Backfill) and Approvals (an effect's destructive step);
`review/` the effort page and its verdict chip; `agent/` the Terminal shell; `components/` the github example's PR Lifetimes (frame,
filters, navigation, approval-gated `invoke`, and a self-navigation off
this machine sending no request) and the triage example (its real `todos` collector
over the `sorter` provider, decisions from the board and the `todos` page,
surviving a reload).

`pages/` is the page-mechanics suite: each spec installs a tiny extension
of its own (`installExtension` in `support/pages.ts`) and checks one generic
behaviour: select, expand, group fold, switch, form, slot, replacement,
embed, component, panel, params. A page migrated into an extension gets a
per-page spec only for behaviour the `specs/pages/*` suite does not cover
(a command's effect, a data flow, keyboard, drag, undo). A spec that only
checks that a page draws, a row opens a page, a group folds or a renderer
takes a URI repeats that suite, and costs a full run for nothing.

What stays a hand walk: the WKWebView window itself, shell-only surfaces
(project setup, the native menu, the clipboard, external URLs), OAuth
sign-in.

## Scripts

- **`profile-renderer.mjs`** — CDP V8 CPU profile of the renderer, against an
  app you bring up by hand (a daemon plus `vite` with `VITE_OXPLOW_REMOTE`).
  The daemon refuses a page without its UI token, so `APP_URL` carries it:
  `APP_URL='http://localhost:5199/#oxplow-token=<token>'` (the token the
  daemon was given with `--token-stdin`).
  `CLICK_TESTID=rail-section-toggle-core:work` expands a collapsed section
  first, so you don't profile an unmounted list by accident.

## Two things to know before trusting a number

**It's Chromium, not WKWebView.** The shipped app runs WKWebView. This is a
good proxy for React/JS work and a poor one for paint, scroll and GC. Say which
you measured. (The suite's WebKit project is Playwright's WebKit build, close
to but not the system WKWebView.)

**Rank by what's actually executing.** An idle renderer reports ~100%
`(idle)`/`(program)`. `profile-renderer.mjs` separates those from real JS for
exactly this reason — the equivalent mistake on the Rust side made a profile
read as 90% `__psynch_cvwait` (parked threads) and produced a wrong ranking.
See `.context/performance.md`.

## What it found first time out

The three idle-timer suspects in tsk219 were all wrong (details in
`.context/performance.md`). At idle the renderer executes **0.0%** JS across
20-25s captures. Worth remembering before optimizing a timer because it *looks*
expensive.

Still unmeasured: interaction cost (typing, scrolling a large diff, metric
pages against real data volume) and anything WKWebView-specific.

## Specs that wait in a holder

The specs of pages that live in the turned-off holder extension under
`.oxplow/extensions/`
(Tasks, Board, Backlog, the wiki, Timeline, Explore Data, the history
graph, handoffs, the Dashboards index, the task-views example) are in its
`e2e-specs/` folder, outside the suite. They come back with
their page.
