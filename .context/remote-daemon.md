# Remote daemon mode

Every project backend is an `oxplow-daemon`, including local ones — a
local project window is a client talking to a daemon on 127.0.0.1 (see
[architecture.md](./architecture.md)). "Remote mode" is therefore not a
separate mechanism, just the same client pointed at a daemon on another
machine (e.g. an EC2 dev box). Single-user by design; SSH is the auth
layer. User-facing setup lives in the user docs — this note is the
developer-facing mechanics.

## Pieces

- **`crates/oxplow-rpc`** — transport-neutral command cores +
  `rpc_dispatch!` registry (`dispatch(name, camelCase JSON args,
  &Services) -> JSON`). Single source of truth for the command set;
  both hosts call it. No `tauri` deps, ever — the daemon builds
  headless.
- **`crates/oxplow-daemon`** — headless binary. `--project <dir>`
  (or `OXPLOW_PROJECT_DIR`) + `--bind 127.0.0.1:7420`. Boots
  `Services::boot` → `oxplow_app::boot::run_boot_orchestration`
  (shared with the Tauri shell — recovery, ensure_primary, all
  watchers/indexers) → control-plane spawn (hooks/MCP for agents in
  tmux on that box). Routes: `POST /ipc/:name` (tauri-specta result
  envelope — built by the shared `oxplow_rpc::ipc_envelope`, the single
  Rust owner of the `{status, data|error}` shape; the Tauri path reaches
  the byte-identical shape via the TS `typedError` wrapper + the same
  `IpcError`, so the two hosts can't diverge on error mapping),
  `GET /events` (WebSocket multiplexing
  `{channel:"oxplow"|"lsp"|"terminal"|"acp", payload}` with the exact
  payload shapes the Tauri bridges emit; the `lsp` frame carries
  `LspSessionEvent` from `LspSessionManager` — see `.context/lsp.md`;
  the `acp` frame carries `AcpEvent` from `Services.acp` — see
  `.context/agent-model.md` → "ACP agents"),
  `GET /health`. Same per-project instance lock as the shell.
  `/ipc` and `/events` require the UI token (see "Auth" below);
  `/health` doesn't. CORS stays permissive so the frontend can run in a
  plain browser (Playwright, a served `dist/`): a page can't obtain the
  token and no cookies are involved, so it exposes nothing.
- **Facade guard** — `@tauri-apps/*` may only be imported under
  `apps/desktop/src/tauri-bridge/`; everywhere else funnels native
  access through a bridge module (e.g. `nativeDialog.ts` wraps the OS
  folder picker). `tauri-bridge/no-tauri-imports.test.ts` fails `bun
  test` on any violation, so a native assumption can't leak past the
  switchable transport and break the browser path silently. (The repo
  has no ESLint; this source-scan guard delivers the invariant in the
  existing test step.)
- **`apps/desktop/src/tauri-bridge/transport.ts`** — the frontend
  switch. With no daemon base it delegates to `@tauri-apps/api`; with
  one it fetches `/ipc/:name` and demuxes the `/events` WS with backoff
  reconnect.

  **Which daemon is a per-window question** (tsk258). The shell injects
  `window.__OXPLOW__ = { base, kind, projectDir }` into every window it creates
  (`src-tauri/src/windows.rs`, `initialization_script` — built through
  `serde_json` so a URL can't break out of the literal), so two project
  windows in one shell process can drive two different daemons.
  `resolveBase` takes it from, in order: localStorage
  `oxplow.remoteBase` (the launcher's manual connect — an explicit user
  action outranks the default), the injected base, then
  `VITE_OXPLOW_REMOTE` (dev override). All three are read once at module
  load — switching is a window reload.

  **Routing is per command.** `invokeRoute(name, base, tauriAvailable)`
  sends shell commands (windowing, native menus, clipboard, project
  lifecycle — no daemon serves them) to Tauri IPC and everything else to
  this window's daemon. The table is
  `generated/shellCommands.ts`, emitted from
  `oxplow_tauri_ipc::SHELL_ONLY_COMMANDS` by the `export_ts_bindings`
  test and asserted by the surface-parity test, so the TS side can't
  drift from the Rust definition. One exception keeps the browser path
  working: with **no Tauri host at all** (plain browser over a tunnel)
  shell commands fall through to the daemon, which answers with a
  structured "unknown command" instead of the renderer throwing on a
  missing `__TAURI_INTERNALS__`.

  Every channel
  `listen()` accepts is declared in `channels.ts`'s `CHANNEL_ROUTING`
  registry with a routing class (`multiplexed` = daemon WS in remote /
  Tauri bus locally; `shellLocal` = Tauri bus only). `listen()`'s
  channel arg is the `ListenChannel` union of those keys, so a new
  channel can't be subscribed without being classified (compile error),
  and `listenRoute` switches on the class. Shell-local channels (e.g.
  `menu:command`) still use the Tauri bus in remote mode, but go inert
  (`listenRoute` → `"none"`) when no Tauri host exists — i.e. the
  frontend running in a plain browser for Playwright-driven testing or
  a served `dist/`. The backoff loop keeps the registered channel
  handlers across a drop, so it auto-re-subscribes
  on reconnect. A reconnect *after* a drop (not the first connect) also
  fires the `onRemoteReconnect` handlers, so consumers re-hydrate the
  snapshot they hold and catch up on events missed while the socket was
  down. `triggerRemoteResync()` fires the same handlers manually (used by
  the daemon health-probe recovery path in `App.tsx`). The backoff loop
  is the WS-transport half; the in-place store recovery it drives is the
  next bullet.
- **Auto-resync on reconnect.** Recovery re-hydrates the client stores
  in place — *no* manual full-page reload. The top-level loader
  (`loadInitialAppState` in `App.tsx`: streams, current stream + its
  threads, workspace context, selected-thread work), the core-store
  subscriptions (`useBackendSubscriptions`: backlog, config, agent
  statuses), and comment threads (`useCommentsForTarget`) all register
  `onRemoteReconnect` handlers. The daemon health probe (App.tsx, 2s
  `/ipc/ping` poll) used to `window.location.reload()` on recovery;
  it now `triggerRemoteResync()`s instead, so an HTTP-level recovery
  takes the same in-place resync path (a reload would drop unsaved
  editor drafts). To make a new surface live-again after a drop,
  register an `onRemoteReconnect` re-fetch alongside its event
  subscription.
- **Launcher connect flow** — `launcher/Launcher.tsx`
  `RemoteConnectSection` + `launcher/remoteRecents.ts`. Probes
  `/ipc/ping` before committing. `Root.tsx` renders the full app
  shell for any window without a shell-assigned `kind` — which is what a
  plain browser over a tunnel is.
- **`components/RemoteConnectionBanner.tsx`** — fixed top strip in
  remote mode: red "reconnecting…" while the WS is down (with
  Disconnect). Once it recovers, state auto-resyncs (see above), so the
  banner shows only a brief, non-blocking "Connection restored — state
  resynced" confirmation that auto-dismisses (`RESTORED_AUTO_DISMISS_MS`)
  — no reload prompt. tmux agents on the daemon box run through the gap.
  (A genuine version/schema skew after a backend upgrade would still
  warrant a reload prompt; there's no skew detection yet, so nothing
  surfaces one today.)

## Deployment model (v1)

Daemon binds loopback only; reach it with
`ssh -L 7420:127.0.0.1:7420 <host>`. No TLS. Multi-user is explicitly
out of scope.

## Auth (tsk345)

Loopback is not a boundary: the agents the daemon runs, the sources it
sandboxes (localhost is allowed) and any web page the person visits can
all reach 127.0.0.1. So `/ipc` and `/events` require a per-launch **UI
token**; only the person's renderer holds it.

- **Minting:** the shell's `DaemonSupervisor` mints 256 random bits per
  daemon and hands them over on **stdin** (`--token-stdin`). Never argv,
  the environment or a file: all are readable by other processes of the
  user, the daemon's own agents included.
- **Hand-started daemon:** it mints its own and prints `ui token: …`.
- **Presenting it:** `Authorization: Bearer <token>` on `/ipc`, and
  `?token=` on the `/events` WebSocket (browsers can't set its headers).
  The comparison is constant-time.
- **Where the renderer gets it** (`transport.ts` `resolveToken`), paired
  with whichever source supplied the base:
  - the launcher's manual connect: a token field, or
    `http://host:port#oxplow-token=…`, saved as `oxplow.remoteToken`;
  - the shell's injected `window.__OXPLOW__.token`;
  - `VITE_OXPLOW_REMOTE_TOKEN` in dev.
  A `#oxplow-token=` URL fragment wins (scripted and Playwright
  sessions).
- **Headless dev:** start the daemon, read its `ui token:` line, and run
  vite with `VITE_OXPLOW_REMOTE=… VITE_OXPLOW_REMOTE_TOKEN=…`.

**Ungated: `/health` and custom component bundles** (P6b.D3,
`oxplow-daemon/src/components.rs`). `GET
/components/{ext}/{component}/{*path}` (`?stream_id=` picks the
worktree; the bare folder URL redirects to the trailing-slash form so a
bundle's relative URLs resolve inside it) serves a private, enabled
extension's declared component bundle for the sandboxed frame a
`viz: custom` lens renders. It can't take the token — a frame can't
carry it — and needn't: it serves the extension's own files, never
project data (the frame reaches that only through the host's bridged
RPCs). It is mounted **outside the permissive CORS layer**, so a web
page can't read a bundle with `fetch`. 404 for a missing, disabled or
bundled extension, an undeclared component, or anything but a plain file
inside the bundle (`safe_bundle_path`: no `..`, `.`, empty, backslash or
NUL segment, no absolute path, no symlink or directory, and resolved
inside the bundle); 413 past the size cap. Every 200 has `Content-Type`
from a small table (`content_type_for`), `X-Content-Type-Options:
nosniff`, `Cache-Control: no-store`, `Referrer-Policy: no-referrer` and
a CSP that lets the bundle load only its own files — `default-src
'none'`, scripts, styles, images and fonts from `'self'` and the bundle
folder's URL (a sandboxed frame's origin is opaque), `connect-src
'none'`, `form-action 'none'`, `base-uri 'none'` (`bundle_csp`). The
main window's CSP (`tauri.conf.json`) has `frame-src http://127.0.0.1:*`
for it, pinned by `tauriCsp.test.ts`.

## Dispatch context

`dispatch` takes an `RpcContext { services, plugin_runtime }`
(`crates/oxplow-rpc/src/lib.rs`; derefs to `Services`). The
`rpc_dispatch!` registry has two sections: `svc { … }` cores receive
`&Services` (the ~180 common commands) and `ctx { … }` cores receive
`&RpcContext` (today just `open_terminal_session`, whose agent path
reads `plugin_runtime` — the control-plane hook/MCP URLs + token).
Both hosts populate it from their own control plane: the Tauri
wrapper from the managed `PluginRuntimeState`, the daemon in
`main.rs` from its `ControlPlane` handle. A host that passes
`plugin_runtime: None` degrades cleanly — plain shell terminals
work, agent spawn returns INVALID.

## Known v1 gaps

- **Picking a different project** = restarting the daemon with a
  different `--project` (it's project-scoped, like the shell's
  shell owns windows, daemons own projects). No remote directory browser.
- External-URL tabs and the native menu/clipboard stay local to the
  shell in both modes.
