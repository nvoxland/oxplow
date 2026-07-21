# Archived Electron e2e suite

These 35 Playwright probes exercised the original Electron build of
oxplow via `playwright._electron.launch(...)`. They do **not** work
against the Tauri 2 build that replaces it: Playwright has no first-
party Tauri driver, the `<webview>` tag flows are different, and
several probes pass arguments (`--user-data-dir`, `--project`) that
the Tauri shell doesn't accept.

The directory is preserved under `tests-e2e.electron-archive/` so
that:
- The hand-written probe steps (page selectors, assertions, fixture
  flows) are still readable as a behavior corpus when porting.
- `git blame` history on the probes is not lost.

## Path forward — superseded, see `tests-e2e/`

This section used to list three Tauri e2e options (`tauri-driver` +
WebdriverIO, CDP into a dev-build webview, a hand-rolled HTTP probe
harness) and note that none had been built. **Remote-daemon mode is a
fourth and better one, and it now works**: point vite at
`oxplow-daemon` via `VITE_OXPLOW_REMOTE` and the real React UI runs in
plain headless Chromium under ordinary Playwright — no driver, no
wdio, no CDP attach.

Verified 2026-07-21: boots with 0 page errors and 0 failed requests.
See `tests-e2e/README.md` for how to bring it up.

The trade-off is that it's **Chromium, not the shipped WKWebView** — it
exercises the app's JS, IPC and rendering logic but not WebKit-specific
paint/scroll behaviour, and it doesn't cover the Tauri shell itself
(windows, menus, native dialogs). For most probes here that's fine;
they assert on UI behaviour, not on WebKit.

## Removing this directory

**Not yet.** A harness exists at `tests-e2e/`, but none of these 35
probes have been ported to it — the new directory holds a boot check
and a profiling script. Their selectors are stale (that build's DOM is
gone), so porting means rewriting against the current `data-testid`s,
which `tests-e2e/boot-check.mjs` dumps for you.

Delete this directory once the probes worth keeping have been ported
or consciously dropped — not merely because a harness exists.
