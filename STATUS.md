# Tauri-migration status

Honest, reviewable feature matrix for the Electron → Tauri 2 + Rust
backend migration. Replaces the optimistic narrative in
`MIGRATION_REVIEW2.md` (gitignored) with a checklist of what works,
what's stubbed, and what's gone. Update this file alongside any
change that flips a row.

## Backend (Rust crates)

| Subsystem | State | Notes |
|---|---|---|
| Stream / thread / work-item lifecycle | ✅ working | `oxplow-app` orchestration, `oxplow-db` stores, full CRUD via Tauri commands. |
| Git ops (sync/refs/blame/branch changes/scopes/search) | ✅ working | `oxplow-git`. Per-stream worktree resolution wired through every git command via `resolve_repo_dir`. |
| Snapshots + content-addressed blob store | ✅ working | `crates/oxplow-app/src/blob_store.rs`; `restore_file_from_snapshot` end-to-end. |
| Hook event ingest + agent-turn lifecycle | ✅ working | `oxplow-runtime`. |
| Agent panes (tmux orchestration) | ✅ working | `oxplow-tmux` with copy-mode helpers. |
| Code-quality scans | ✅ working | In-process metrics + duplication detection (tree-sitter); findings store. |
| LSP session manager + 4 LSP MCP tools | ✅ working | `oxplow-lsp`; `oxplow-app::lsp_sessions`. |
| Daemon recovery on boot | ✅ working | `oxplow-app::recovery`. |
| MCP server | ✅ working | 38 tools via rmcp. |
| Tauri command surface | ✅ working | 156 commands across `crates/oxplow-tauri-ipc/src/commands/`. |

## Renderer (Tauri frontend)

| Subsystem | State | Notes |
|---|---|---|
| File-session state (open files / dirty tabs / LRU) | ✅ working | `apps/desktop/src/editor-session.ts` — restored from `main` with 9 unit tests. |
| Editor pane (Monaco + LSP markers + blame) | ✅ working | Reads bindings shapes directly: `BlameLine.author_time`, `LocalBlameEntry.git`. |
| Terminal pane (xterm + tmux attach) | ✅ working | `open_terminal_session` / `forward_terminal_input` / `close_terminal_session` Tauri commands; `terminal:event` channel. Tmux history-mode messages dispatch through `oxplow-tmux::copy_mode_*`. |
| LSP bridge (per-language client) | ✅ working | `open_lsp_client` / `send_lsp_message` / `close_lsp_client` Tauri commands; `lsp:event` channel. Echo-server round-trip test in `oxplow-app::lsp_clients`. |
| Native menu (macOS/Windows) | ✅ working | `set_native_menu` translates `MenuGroupSnapshot[]` → `tauri::menu::Menu`; `menu:command` event re-emits activations to renderer. |
| External-URL tabs | ✅ working | `WebviewWindow` spawn via `open_external_url`; sandboxed by the `external-url` capability with **zero** oxplow commands and zero plugin permissions. |
| `getChangeScopes` staged/unstaged | ✅ working | `oxplow-git::collect_working_tree_changes` populates both arrays from `git status --porcelain`. |
| `createStream` / new-stream form | ✅ working | Maps "existing" / "new" / "worktree" source modes to `create_worktree` and `adopt_worktree` Tauri commands. |
| Per-stream git scoping | ✅ working | All 22 git/log commands accept `Option<String> stream_id` and resolve the active worktree via `SqliteStreamStore::list`. |
| Editor focus tracking | 🟡 no-op | The renderer pushes editor focus to `desktopBridge().updateEditorFocus`; the bridge currently swallows it because the daemon doesn't consume editor focus yet. Harmless. |
| `legacy-bridge.ts` / `legacy-*` filenames | ✅ gone | All renamed (`api-types.ts`, `editor-session.ts`); `window.oxplowApi` global eliminated. |
| `buildDesktopAdapter` Proxy + `notPorted` runtime | ✅ gone | Replaced by a 13-method typed `DesktopBridge` facade. Missing methods are now compile errors, not deferred runtime crashes. |

## Tooling / packaging

| Item | State | Notes |
|---|---|---|
| `tauri-specta` v2 binding generation | ✅ working | `cargo test -p oxplow-tauri-ipc` regenerates `apps/desktop/src/tauri-bridge/generated/bindings.ts`. |
| Bindings drift guard in CI | ✅ working | `.github/workflows/ci.yml` "Verify generated TS bindings are up to date" step fails the PR on a non-empty `git diff` after regeneration. |
| `cargo-llvm-cov` workspace coverage in CI | ✅ working | Workspace floor: 65% lines; current baseline ~72.3% lines / 69.2% regions / 59.2% functions. Per-crate floors enforced by `scripts/coverage-floors.py` (e.g. oxplow-runtime ≥90%, oxplow-git ≥80%, oxplow-app ≥75%, oxplow-tauri-ipc ≥12%). |
| Capabilities listed explicitly in `tauri.conf.json` | ✅ working | `app.security.capabilities = ["main-window", "external-url"]`. |
| External-URL capability targets webview labels | ✅ working | `external-url.json` uses `webviews: ["ext-url-*"]` (more precise than the parent-window label pattern). |
| `shell:default` replaced with allowlist | ✅ working | tmux + git + typescript-language-server in `main-window.json`. |
| CSP set in `tauri.conf.json` | ✅ working | `unsafe-inline` retained for styles only (Monaco needs it). |
| Auto-update signing key | ❌ deferred | Operational; needs cert generation + secret wiring. Blocks shipping signed updates. Punch-list in `ideas/signing-and-auto-update.md`. |
| macOS / Windows code signing | ❌ deferred | Same — needs Apple Developer cert + Windows EV cert + CI secret integration. Punch-list in `ideas/signing-and-auto-update.md`. |
| `oxplow-config` preserves user comments on write | 🟡 partial | Unknown top-level keys are preserved through writes; YAML comments are not (no comment-aware Rust YAML crate). Documented in the `write_project_config` docstring. |

## Test counts (per `cargo test`)

| Crate | Tests | LOC | LOC/test |
|---|---|---|---|
| `oxplow-git` | 54 | 2,925 | 54 |
| `oxplow-app` | 54 | 3,286 | 61 |
| `oxplow-db` | 46 | 4,321 | 94 |
| `oxplow-runtime` | 29 | 1,051 | 36 |
| `oxplow-session` | 17 | 785 | 46 |
| `oxplow-domain` | 17 | 822 | 48 |
| `oxplow-config` | 12 | 514 | 43 |
| `oxplow-lsp` | 7 | 542 | 77 |
| `oxplow-pty` | 4 | 531 | 133 |
| `oxplow-tmux` | 3 | 371 | 124 |
| `oxplow-fs-watch` | 3 | 178 | 59 |
| `oxplow-tauri-ipc` | 39 | 2,820 | 72 |
| `oxplow-mcp` | 10 | 1,161 | 116 |

Total: 295 tests. `oxplow-tauri-ipc` got a `tauri::test` integration
harness (`tests/commands.rs` + `tests/harness.rs`) that exercises 27
representative commands across 17 of the 24 command modules — the
seam REVIEW3/4 wanted protected (state plumbing, argument shapes,
error mapping). The remaining 0%-coverage modules (agent_panes,
branch, effort, log, lsp, menu, notes, terminal, webview) need real
git/tmux/pty fixtures and are deferred. `oxplow-mcp` keeps its 10
tool-handler smoke tests. `oxplow-tmux` and `oxplow-pty` stay thin —
both are subprocess-driven and well-exercised by the `oxplow-app`
integration paths.

## Frontend tests

`apps/desktop/src/editor-session.test.ts` — 9 unit tests for the
file-session state machine. Other renderer tests are colocated
`*.test.ts` files exercised via `bun test`. End-to-end: the
Electron-era Playwright suite at `tests-e2e.electron-archive/`
**does not** work against the Tauri build (see that directory's
README). No Tauri e2e harness exists yet.

## Still open (deferred)

- **Auto-update signing key + macOS/Windows code signing.** Blocks
  shipping signed installers. Operational, not code. Detailed
  punch-list in `ideas/signing-and-auto-update.md`.
- **Tauri e2e harness.** Three plausible paths (tauri-driver +
  WebdriverIO, CDP-into-webview via Playwright, hand-rolled HTTP
  probes). None built; see `tests-e2e.electron-archive/README.md`.
  The integration tests in `oxplow-app/` plus the new
  `oxplow-tauri-ipc` and `oxplow-mcp` smoke suites cover the
  business-logic surface that matters; the gap is renderer-side
  click-through coverage.
- **Per-crate coverage floors.** Workspace floor of 65% lines is
  in place; raising it as `oxplow-tauri-ipc` / `oxplow-mcp`
  coverage grows is the lever. The thinnest crates today are the
  command-adapter modules (each `#[tauri::command]` is a thin
  wrapper that needs an integration harness to exercise; unit
  tests can only cover the IpcError mapping and helpers).

## How to verify

```sh
# Rust
cargo test --workspace
# expected: 295 passed, 0 failed

cargo llvm-cov --workspace --summary-only --fail-under-lines 65
# expected: passes; current baseline ~72.3%

cargo llvm-cov report --json | python3 scripts/coverage-floors.py
# expected: every crate clears its per-crate floor

# Frontend
cd apps/desktop && bun run typecheck
# expected: clean exit

bun test apps/desktop/src/editor-session.test.ts
# expected: 9 pass

# Bridge wiring
cargo check -p oxplow-desktop
# expected: clean

# No "not yet ported" stubs in api.ts
grep -nE 'not yet ported|not yet wired' apps/desktop/src/api.ts
# expected: no matches
```
