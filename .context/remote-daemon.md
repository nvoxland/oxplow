# Remote daemon mode

The desktop shell can drive a backend running on another machine
(e.g. an EC2 dev box) instead of its in-process backend. Single-user
by design; SSH is the auth layer. User-facing setup lives in the user
docs — this note is the developer-facing mechanics.

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
  envelope), `GET /events` (WebSocket multiplexing
  `{channel:"oxplow"|"lsp"|"terminal", payload}` with the exact
  payload shapes the Tauri bridges emit; the `lsp` frame carries
  `LspSessionEvent` from `LspSessionManager` — see `.context/lsp.md`),
  `GET /health`. Same per-project instance lock as the shell.
  CORS is fully permissive (`CorsLayer::permissive()`) so the
  frontend can run in a plain browser (Playwright-driven UX testing,
  a statically served `dist/`); loopback bind + SSH is the auth
  layer, so origin checks add nothing. Revisit if a direct-expose
  mode lands.
- **Facade guard** — `@tauri-apps/*` may only be imported under
  `apps/desktop/src/tauri-bridge/`; everywhere else funnels native
  access through a bridge module (e.g. `nativeDialog.ts` wraps the OS
  folder picker). `tauri-bridge/no-tauri-imports.test.ts` fails `bun
  test` on any violation, so a native assumption can't leak past the
  switchable transport and break the browser path silently. (The repo
  has no ESLint; this source-scan guard delivers the invariant in the
  existing test step.)
- **`apps/desktop/src/tauri-bridge/transport.ts`** — the frontend
  switch. Local mode delegates to `@tauri-apps/api`; remote mode
  (localStorage `oxplow.remoteBase`, set by the launcher's connect
  flow; dev override `VITE_OXPLOW_REMOTE`) fetches `/ipc/:name` and
  demuxes the `/events` WS with backoff reconnect. Mode is read once
  at module load — switching is a window reload. Shell-local event
  channels (e.g. `menu:command`) still use the Tauri bus in remote
  mode, but go inert (`listenRoute` → `"none"`) when no Tauri host
  exists — i.e. the frontend running in a plain browser for
  Playwright-driven testing or a served `dist/`.
- **Launcher connect flow** — `launcher/Launcher.tsx`
  `RemoteConnectSection` + `launcher/remoteRecents.ts`. Probes
  `/ipc/ping` before committing. `Root.tsx` renders the full app
  shell whenever a remote base is set.
- **`components/RemoteConnectionBanner.tsx`** — fixed top strip in
  remote mode: red "reconnecting…" while the WS is down (with
  Disconnect), accent "reload to resync" once it recovers. Reload is
  user-initiated, not automatic — a reload drops unsaved editor
  drafts. tmux agents on the daemon box run through the gap.

## Deployment model (v1)

Daemon binds loopback only; reach it with
`ssh -L 7420:127.0.0.1:7420 <host>`. No TLS, no tokens — adding a
bearer check behind a flag is the designated extension point for a
later direct-expose mode (mirror the control-plane's `hook_token`
pattern). Multi-user is explicitly out of scope.

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
  process-per-window model). No remote directory browser.
- External-URL tabs and the native menu/clipboard stay local to the
  shell in both modes.
