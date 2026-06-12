# Developing for Oxplow

## Prerequisites

- **Bun 1.3.9** and **Node 22.13.1** (frontend toolchain).
- **Rust stable (≥ 1.80)** — `rust-toolchain.toml` pins it; rustup
  installs the right version automatically.
- **Platform Tauri deps**:
  - macOS: `xcode-select --install` (Xcode CLT).
  - Linux: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev
    librsvg2-dev patchelf build-essential`.
  - Windows: WebView2 (preinstalled on modern Windows; Microsoft
    redistributable otherwise) + MSVC build tools.
- **Git** — oxplow's git features expect the workspace root to be a
  repo.
- **`tmux`** — the agent panes are tmux-managed. Optional for tests
  (the runtime tmux suite skips when tmux isn't on PATH).

If you use [mise](https://mise.jdx.dev/), `mise install` picks up
bun/node/rust from `mise.toml`.

## Install

```
bun install --frozen-lockfile
```

This installs only the frontend deps (React, Monaco, xterm,
@tauri-apps/api). Cargo handles Rust deps lazily on first build.

## Run from source

```
./bin/oxplow
```

`bin/oxplow` treats the current working directory as the project
root. Oxplow's workspace isolation rule (see
[.context/architecture.md](./.context/architecture.md)) keeps it
from climbing into a parent repo.

The launcher dispatches by what's already built:

1. `target/release/oxplow` if present — production binary, embedded
   frontend, no Vite needed.
2. `target/debug/oxplow` if present — debug binary; **expects Vite
   running on `http://localhost:5173`** (Tauri reads `devUrl` for
   debug profile and only embeds `frontendDist` in release builds).
3. Falls back to `cargo tauri dev` from `apps/desktop/` (requires
   `cargo install tauri-cli`).

### Recommended dev loop (rapid iteration)

The fastest loop avoids rebuilding the Rust shell unless Rust code
changed. Run two long-lived processes:

```
  # Option A (default): tauri-cli dev. One command, one terminal.
  # Starts Vite, builds + runs the debug binary, watches Rust and TS,
  # auto-restarts the window on Rust changes. Use this unless you have
  # a reason not to.
  bun run tauri:dev                       # from repo root (defined in top-level package.json)

  # Option B: split-process dev — escape hatch when A's auto-rebuild
  # is in your way (e.g. you want to decide when Rust rebuilds, or
  # tauri-cli's watcher is misbehaving). Same debug binary, same Vite,
  # just driven manually.
  #
  # terminal 1 — frontend dev server (HMR, ~150ms reloads on TS save)
  cd apps/desktop && bun run dev          # vite on :5173

  # terminal 2 — debug binary; talks to the running vite server above
  cargo build -p oxplow-desktop && ./bin/oxplow

  # Option C: production-style binary (no Vite running; embeds dist/).
  # Requires --release because debug Tauri builds always use devUrl —
  # plain `cargo build` + ./bin/oxplow gives a blank window.
  bun run --cwd apps/desktop build
  cargo build --release -p oxplow-desktop && ./bin/oxplow
```

Then iterate:

- **Frontend-only change (`apps/desktop/src/**`)**: save the file —
  Vite HMR pushes it into the running window. No rebuild, no restart.
- **Rust crate change (`crates/**` or `apps/desktop/src-tauri/**`)**:
  `cargo build -p oxplow-desktop` in terminal 2, then quit the app
  window and re-run `./bin/oxplow`. Cargo's incremental builds make
  this ~5–15s for typical edits.
- **`tauri.conf.json` / capability JSON change**: same as Rust —
  `tauri-build` only re-embeds config when its build script reruns.
  A `cargo clean -p oxplow-desktop` + rebuild forces it.
- **IPC surface change (`#[tauri::command]` signatures, request /
  response types)**: `cargo test -p oxplow-tauri-ipc` regenerates
  `apps/desktop/src/tauri-bridge/generated/bindings.ts`. Commit the
  diff in the same change — CI gates on it.

### Gotchas

- **Blank window** usually means the debug binary couldn't reach
  Vite. Confirm `curl -sI http://localhost:5173/` returns 200; if
  not, start `bun run dev` first.
- **`tauri-build` doesn't re-embed `frontendDist` automatically** —
  it caches across debug builds. If a release-mode build picks up
  stale assets, `cargo clean -p oxplow-desktop` and rebuild.
- **Bare-DB boot** (no streams / threads) is normal on a fresh clone.
  The desktop shell auto-creates the primary stream and seeds a
  default thread on first launch.
- **Agent pane "can't find session"**: that's tmux mode trying to
  attach to a session that doesn't exist yet. The default transport
  is `direct` (spawns the agent CLI in a PTY, no tmux); switch back
  via the agent pane's kebab → "Use direct mode" if you toggled it.
- **Vite must be running for any debug build** even if you're only
  iterating on Rust. Killing Vite and re-running the binary is
  what produces the empty white window.

## Run headless (daemon + browser)

The full app also runs without the Tauri shell: `oxplow-daemon`
serves the backend over HTTP/WebSocket, and the frontend in remote
mode talks to it from any browser. This is the route for driving the
UI with Playwright (no Tauri driver exists for macOS) and for remote
dev (see `.context/remote-daemon.md`).

```
  # terminal 1 — headless backend on loopback
  cargo build -p oxplow-daemon && ./target/debug/oxplow-daemon --project . --bind 127.0.0.1:7420

  # terminal 2 — frontend in remote mode (vite on :5173)
  VITE_OXPLOW_REMOTE=http://127.0.0.1:7420 bun run --cwd apps/desktop dev
```

No HMR wanted? Serve a static production build instead of the dev
server — either bake the remote base in at build time:

```
  VITE_OXPLOW_REMOTE=http://127.0.0.1:7420 bun run --cwd apps/desktop build
  bun run --cwd apps/desktop preview      # serves dist/ on :4173
```

or build plain (`bun run --cwd apps/desktop build`), serve `dist/`
with any static server, and use the launcher's Remote connect flow —
it stores `oxplow.remoteBase` in localStorage at runtime, no baked-in
URL.

Then open `http://localhost:5173` in a browser (or point Playwright
at it). Notes:

- The project dir must already contain `.oxplow/`; sanity-check the
  daemon with `curl http://127.0.0.1:7420/health`.
- `VITE_OXPLOW_REMOTE` flips the frontend transport switch
  (`apps/desktop/src/tauri-bridge/transport.ts`) into remote mode at
  dev/build time; without it the frontend expects Tauri IPC and a
  browser tab won't boot.
- The daemon takes the same per-project instance lock as the desktop
  shell — the app and the daemon can't run against the same project
  simultaneously.
- Shell-native surfaces (native menus, window chrome, Tauri dialogs)
  don't exist in this mode; everything else is the real app.

## Test

```
bun run test     # runs both Rust and TS suites
cargo test --workspace
bun run --cwd apps/desktop test
```

`cargo test --workspace` is the Rust suite (≈100 tests across the
backend crates). It also regenerates `apps/desktop/src/tauri-bridge/
generated/bindings.ts` via the `oxplow-tauri-ipc` `export_ts_bindings`
test — CI fails if `git diff` of that file is non-empty after the
test run.

Frontend tests still use `bun test` (run from `apps/desktop/`).
The original Electron-era Playwright suite lives under
`tests-e2e.electron-archive/` and **does not** work against the
Tauri build; see that directory's README for the path forward.
There is no current Tauri e2e harness.

### Coverage

CI runs `cargo llvm-cov --workspace --summary-only` on every PR and
uploads an `lcov.info` artifact. To reproduce locally:

```
cargo install cargo-llvm-cov   # one-time
cargo llvm-cov --workspace --summary-only   # per-crate line %
cargo llvm-cov --html --workspace           # HTML report under target/llvm-cov/html
```

No coverage floor is gated yet — the goal is to keep the numbers
visible so the thinnest crates (`oxplow-mcp`, `oxplow-tauri-ipc`,
`oxplow-pty`, `oxplow-tmux`) get backfill before regressions creep
in.

## Build installers

```
bun run tauri:build
```

Runs Vite + cargo to produce platform installers in
`target/release/bundle/`:

- macOS: `.dmg` + `.app.tar.gz` (arm64 and x64)
- Windows: `.msi` / `.exe`
- Linux: `.deb` + `.AppImage`

Builds are unsigned in CI. Add signing certs by setting Tauri's
standard signing env vars; see Tauri docs for `TAURI_PRIVATE_KEY` and
the per-platform keychain integration.

## Documentation site

User-facing docs live under `docs/` and are built with MkDocs
Material — unchanged from the pre-rewrite setup.

Prereqs: Python 3.11+ and [Poetry](https://python-poetry.org/) 2.x.

```
poetry install --with docs
poetry run mkdocs serve         # live preview at http://localhost:8000
poetry run mkdocs build --strict  # one-shot build into site/
```

## CI

`.github/workflows/ci.yml`:

1. **test** (ubuntu-latest) — `bun install`, `bun run typecheck`,
   `cargo test --workspace`, ts-bindings drift guard, `cargo fmt
   --check`, `cargo clippy -- -D warnings`.
2. **package** (matrix: ubuntu, macOS, Windows) — `bun run
   tauri:build`, uploads installer artifacts.

Cargo registry + target dir cached per OS, keyed on `Cargo.lock`.

## Codebase map

- `apps/desktop/` — Tauri 2 desktop product (frontend + shell).
- `apps/desktop/src/` — frontend TS (React/Monaco/xterm).
- `apps/desktop/src/tauri-bridge/` — typed facade over
  `@tauri-apps/api`; UI imports from here, not `@tauri-apps/api`
  directly.
- `apps/desktop/src-tauri/` — Tauri shell crate, `tauri.conf.json`,
  `capabilities/`.
- `crates/` — reusable Rust libraries:
  - `oxplow-domain` — pure types + store traits.
  - `oxplow-db` — rusqlite stores + migrations.
  - `oxplow-config` — YAML config load/validate.
  - `oxplow-fs-watch` — debounced notify wrapper.
  - `oxplow-git` — repo detection, branches, worktrees, conflict state.
  - `oxplow-session` — stream + worktree lifecycle.
  - `oxplow-runtime` — write guard + filing enforcement.
  - `oxplow-tmux` — tmux command wrapper.
  - `oxplow-pty` — owner-task PTY manager (portable-pty).
  - `oxplow-lsp` — JSON-RPC stdio proxy.
  - `oxplow-mcp` — MCP server (rmcp).
  - `oxplow-app` — Services orchestration.
  - `oxplow-tauri-ipc` — `#[tauri::command]` adapters + tauri-specta
    TS-binding export.

Subsystem docs live under [`.context/`](./.context/). Path
references inside point at the current Rust crate layout.

## Capability schema files

`apps/desktop/src-tauri/capabilities/` references
`gen/schemas/<platform>-schema.json` so editors (VS Code,
JetBrains) autocomplete permission identifiers. Those schemas are
regenerated by `tauri-build` on every `cargo build` and are
gitignored. On a fresh clone, your IDE will report
`unresolved $schema` on the capability files until you run
`cargo build` once.

## Conventions

- **Commit messages**: subject line, blank line, bullet list. Never
  `--amend` a shipped commit.
- **Tests**: real DB (`Database::in_memory()` or tempfile-backed),
  real SQLite, no mocking.
- **Work items as durable records**: every Edit/Write to project
  files needs a tracked task. See CLAUDE.md for filing rules.
