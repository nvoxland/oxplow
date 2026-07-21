# App-level harness: the real UI in a real browser

Drives the actual React app in headless Chromium against `oxplow-daemon`,
using the `VITE_OXPLOW_REMOTE` transport switch. Standard Playwright — no
`tauri-driver`, no WebdriverIO.

`tests-e2e.electron-archive/README.md` lists three "paths forward" for Tauri
e2e and says none had been built. It predates **remote-daemon mode**, which is
a fourth and better one: `.context/remote-daemon.md` says CORS is
`permissive()` specifically so "the frontend can run in a plain browser
(Playwright-driven UX testing)", `listenRoute` degrades shell-local channels to
inert without a Tauri host, and `no-tauri-imports.test.ts` guards the facade so
native assumptions can't leak in.

**That path is verified working** (2026-07-21): the app boots with 0 page
errors and 0 failed requests, renders the rail, file tree, work sections and a
live agent terminal. See `boot-check.mjs`.

This is *not* a port of the archive's 35 probes — those remain a behaviour
corpus to draw on. Keep the archive until they're ported or consciously
dropped.

## Bring it up

```sh
# 1. a throwaway project (never point the daemon at real work — it takes the
#    per-project instance lock and runs full boot orchestration: watchers,
#    indexers, gauges, snapshot capture)
mkdir -p /tmp/oxproj && cd /tmp/oxproj && git init -q .
git commit -q --allow-empty -m init

# 2. daemon (--init creates .oxplow/ on a fresh project)
cargo build -p oxplow-daemon --release
./target/release/oxplow-daemon --project /tmp/oxproj --bind 127.0.0.1:7431 --init

# 3. vite pointed at it. Use a non-default port if you already have a dev
#    server on 5173 — that one has no VITE_OXPLOW_REMOTE and will try to talk
#    to a Tauri host that isn't there.
VITE_OXPLOW_REMOTE=http://127.0.0.1:7431 \
  bun run --cwd apps/desktop dev -- --port 5199 --strictPort

# 4. drive it
APP_URL=http://localhost:5199/ node tests-e2e/boot-check.mjs
```

## Scripts

- **`boot-check.mjs`** — does the UI boot in a browser at all? Reports page
  errors, failed requests, console output, a screenshot, and a dump of every
  `data-testid` the build exposes. That dump is the starting point for writing
  a probe; the archive's selectors are stale.
- **`profile-renderer.mjs`** — CDP V8 CPU profile of the renderer.
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
