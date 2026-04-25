# Developing Oxplow

## Prerequisites

- **Bun 1.3.9** and **Node 22.13.1** (pinned in `mise.toml` — if you use
  [mise](https://mise.jdx.dev/), `mise install` picks them up).
- **A C/C++ toolchain + Python 3** for `node-pty`'s native build.
  - macOS: `xcode-select --install`.
  - Linux: `sudo apt-get install build-essential python3`.
  - Windows: Visual Studio Build Tools (C++ workload) + Python 3.
- **Git**, since oxplow's Git features expect the workspace root to be
  a repo.

## Install

```
bun install --frozen-lockfile
```

This downloads Electron (~100 MB on first run) and compiles `node-pty`
against the Electron node headers.

## Run from source

```
./bin/oxplow
```

The wrapper runs `bun run build` (UI bundle + Electron main + preload),
then launches `electron .` with `--project=$PWD`. Each launch
rebuilds — edits in `src/` land the next time you start.

`bin/oxplow` treats the current working directory as the project root.
Oxplow's workspace isolation rule (see
[.context/architecture.md](./.context/architecture.md)) keeps it
from climbing into a parent repo, so invoke it from the directory you
want it to manage.

## Test

```
bun test
```

That command chains `bunx tsc --noEmit` first (also wired as
`bun run typecheck`), then runs the bun test suite. Unit tests are
colocated with their store / module under `src/`. Cross-store and
Stop-hook behaviour lives in `src/electron/runtime.test.ts`. Tests
use a fresh `mkdtempSync` project directory against a real SQLite
file — no DB mocking.

End-to-end Playwright tests live under `tests-e2e/` and aren't wired
into `bun test`.

## Build installers

```
bun run dist
```

Runs the JS build, then invokes `electron-builder` using the `build`
config in `package.json`. Output lands in `release/`:

- macOS: `.dmg` + `.zip` (arm64 and x64)
- Windows: `.exe` (NSIS, x64)
- Linux: `.AppImage` + `.deb` (x64)

Builds are unsigned — macOS and Windows both surface "unsigned
developer" warnings on install. Adding signing certs means dropping
the `CSC_IDENTITY_AUTO_DISCOVERY=false` workaround in CI and
supplying the usual `CSC_LINK` / `CSC_KEY_PASSWORD` / Windows cert
secrets.

## Documentation site

User-facing docs live under `docs/` and are built with MkDocs
Material. Python deps are managed by Poetry — keep them isolated
from the rest of the repo (the project itself is TypeScript).

Prereqs: Python 3.11+ and [Poetry](https://python-poetry.org/) 2.x.

```
poetry install --with docs
poetry run mkdocs serve         # live preview at http://localhost:8000
poetry run mkdocs build --strict  # one-shot build into site/
```

Markdown content lives under `docs/`. Custom styling is in
`docs/stylesheets/extra.css` and the home-page hero is in
`docs/overrides/home.html`. The `mkdocs.yml`, `pyproject.toml`,
and `poetry.lock` live at the repo root.

`.github/workflows/docs.yml` deploys to GitHub Pages on every push
to `main` that touches `docs/**`. Local builds never push — only
the workflow runs `mkdocs gh-deploy`.

## CI

`.github/workflows/ci.yml` runs on every push to `main` and every
pull request:

1. **test** (ubuntu-latest) — installs, typechecks, runs `bun test`.
2. **package** (matrix: ubuntu, macOS, Windows) — installs, builds
   JS bundles, runs `electron-builder`, uploads each installer set as
   an `oxplow-<os>` artifact (14-day retention).

Both jobs cache `.ci-cache/` (bun install cache, Electron binary,
electron-builder cache) keyed on `runner.os` + `hashFiles('bun.lock')`.

## Codebase map

Subsystem docs live under [`.context/`](./.context/). Read the
relevant one before touching that subsystem — they're short on
purpose:

- [architecture.md](./.context/architecture.md) — hybrid React +
  Monaco shell; workspace isolation rule.
- [data-model.md](./.context/data-model.md) — SQLite tables, stores,
  the single-`sort_index` queue invariant.
- [agent-model.md](./.context/agent-model.md) — Claude launch, the
  Stop-hook loop, MCP tools, write guard.
- [ipc-and-stores.md](./.context/ipc-and-stores.md) — adding a new
  persisted operation end-to-end.
- [theming.md](./.context/theming.md) — CSS variable tiers.
- [git-integration.md](./.context/git-integration.md) — `.git`
  watchers, blame, commit execution.
- [editor-and-monaco.md](./.context/editor-and-monaco.md) — editor
  pane, models, decorations, diff editor, LSP bridge.
- [usability.md](./.context/usability.md) — UI rules (Enter submits,
  Escape cancels, right-click for destructive actions, test-id
  conventions).

Update the matching doc in the same commit as the code change — docs
that drift from code are worse than no docs.

## Conventions

- **Commit messages**: subject line, blank line, bullet list describing
  what changed. See the existing `git log` for style. Never `--amend`
  a shipped commit; new effort → new commit.
- **Tests**: real DB, real SQLite file, no mocking.
- **Work items as durable records**: the project dogfoods itself —
  when the repo's own `.oxplow/` state is pointed at its source, every
  change should be attached to a work item. See CLAUDE.md for the rules.
