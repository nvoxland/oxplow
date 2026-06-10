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
  payload shapes the Tauri bridges emit), `GET /health`. Same
  per-project instance lock as the shell.
- **`apps/desktop/src/tauri-bridge/transport.ts`** — the frontend
  switch. Local mode delegates to `@tauri-apps/api`; remote mode
  (localStorage `oxplow.remoteBase`, set by the launcher's connect
  flow; dev override `VITE_OXPLOW_REMOTE`) fetches `/ipc/:name` and
  demuxes the `/events` WS with backoff reconnect. Mode is read once
  at module load — switching is a window reload.
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

## Known v1 gaps

- **Remote agent spawn**: `terminal::open_terminal_session` needs
  `PluginRuntimeState` (control-plane URLs + token), which only the
  Tauri shell materializes today. The daemon already carries the
  same values in `DaemonState` — wiring a dispatch context that
  includes them is the designed next step. Until then, start agents
  in tmux on the remote box directly.
- **Picking a different project** = restarting the daemon with a
  different `--project` (it's project-scoped, like the shell's
  process-per-window model). No remote directory browser.
- External-URL tabs and the native menu/clipboard stay local to the
  shell in both modes.
