# The browser suite: the real UI in a real browser

`bun run e2e` drives the built React app in headless Chromium against a real
daemon — Playwright (`@playwright/test`), no `tauri-driver`. The frontend
reaches the daemon the way a remote window does: the transport reads its base
and token from localStorage (`oxplow.remoteBase`, `oxplow.remoteToken`), and
CORS is permissive for exactly this (`.context/remote-daemon.md`).

## How a run is put together

- **`playwright.config.ts`** (repo root). Its `webServer` builds the frontend
  once (`vite build`) and serves it with `vite preview` on 127.0.0.1:4173, so a
  spec never meets a stale `dist/`.
- **`support/global-setup.ts`** builds `oxplow-daemon-sim` and
  `oxplow-acp-fake` and hands their paths to the workers
  (`OXPLOW_E2E_DAEMON`, `OXPLOW_E2E_ACP_FAKE`).
- **`support/daemon.ts`** starts one daemon: `oxplow-daemon-sim` (the daemon
  with its secrets in memory — nothing reaches the keychain) over a throwaway
  git project, with its own `OXPLOW_HOME` and `TMUX_TMPDIR`. `ipc()` calls
  `/ipc/<name>` as the person; `run()` runs a bus command, confirmed.
- **`support/fixtures.ts`** — `test` and `expect` for specs:
  - `daemon`, one per worker. Before any page opens it selects an ACP thread
    on the fake agent: the boot thread is a terminal agent's, and the suite
    never starts a real agent CLI.
  - `storageState` points the page at that daemon.
  - `pageErrors` fails any spec whose page threw.
- **`specs/<area>/*.spec.ts`** — the specs. Wait with web-first `expect`, never
  a sleep.

## Scripts

- **`profile-renderer.mjs`** — CDP V8 CPU profile of the renderer, against an
  app you bring up by hand (a daemon plus `vite` with `VITE_OXPLOW_REMOTE`).
  `CLICK_TESTID=rail-section-toggle-work` expands a collapsed section first, so
  you don't profile an unmounted list by accident.

## Two things to know before trusting a number

**It's Chromium, not WKWebView.** The shipped app runs WKWebView. This is a
good proxy for React/JS work and a poor one for paint, scroll and GC. Say which
you measured.

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
